//! Second-plane jobs: long operations (save, recalculate) run off the interface thread with
//! progress and cooperative cancellation (docs/plano-responsividade.md, Fases 1–3).
//!
//! The interface thread only draws and dispatches. A command that measures long becomes a
//! [`Job`]: the UI thread takes a cheap snapshot (`Arc<Workbook>`, a [`SavePlan`]) and the work
//! runs on its own thread, sending `Progress`/`Done` over a channel that [`JobManager::poll`]
//! drains each frame. Outcomes are applied through [`SheetApp::apply_job`] with the guards
//! (same document, workbook unchanged) the plan's revision model asks for.
//!
//! On wasm there are no threads: a job's work runs in the call that spawns it (the web app was
//! already synchronous there and file writes go through the host instead); the API — and so the
//! callers — stay identical for a cooperative per-frame executor to take over.
//!
//! [`SheetApp::apply_job`]: crate::SheetApp::apply_job

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::Context;
use gridcraft_engine::calc::CalcHooks;
use gridcraft_engine::cmd::file::{SaveOutcome, SavePlan};
use gridcraft_engine::model::Workbook;

pub type JobId = u64;

/// The sentinel error a cancelled job reports; the interface dismisses it silently.
pub const CANCELLED: &str = "cancelled";

/// When a command stops being "the interface is quick" and becomes a job. Small workbooks keep
/// the plain inline path (so tests and the control channel see their command complete directly);
/// big ones — the case the plan exists for — go to the background. Tests push these to 0 to
/// force the job path deterministically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    /// `file.save` / `file.saveAs` spawn a job above this many stored cells.
    pub save_cells: usize,
    /// F9 / Shift+F9 spawn a job above this many formula cells.
    pub calc_formulas: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds { save_cells: 50_000, calc_formulas: 30_000 }
    }
}

/// What work a job performs, matched by the interface to apply its outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// `file.save` / `file.saveAs`. After it completes the interface marks `saved` (the snapshot
    /// actually written) and, for full-fidelity formats, the document's own `path`.
    Save { doc_uid: u64, path: String },
    /// F9 (`formulas.calculateNow`, `full`) or Shift+F9 (`formulas.calculateSheet`). The outcome
    /// is applied only while the workbook is still the snapshot the job took.
    Calc { doc_uid: u64, full: bool },
    /// Solver / Goal Seek on a snapshot (Fase 3): the analysis runs on a private session and the
    /// outcome replaces the document's workbook (one undo step) when it is still current.
    Analysis { doc_uid: u64 },
}

/// What a finished job hands back.
pub enum JobResult {
    /// `autosave`: an AutoSave background save — errors log instead of a message box.
    Save { doc_uid: u64, plan: SavePlan, outcome: Result<SaveOutcome, String>, autosave: bool },
    /// The calculation outcome stays behind a `Box`: the recomputed workbook and fresh `Calc`
    /// are large values that would otherwise pad every variant.
    Calc { doc_uid: u64, wb: Arc<Workbook>, full: bool, outcome: Box<Result<CalcOutcome, String>> },
    /// A finished Solver / Goal Seek: the private session's workbook and calculation state, the
    /// command's JSON result, and the snapshot it was taken from (the apply guard).
    Analysis { doc_uid: u64, snapshot: Arc<Workbook>, solver: bool, outcome: Box<Result<AnalysisOutcome, String>> },
}

/// The analysis outcome: the workbook (values, saved model, report sheets), its calculation state
/// and the command's own JSON result (the Solver Results dialog reads it).
pub struct AnalysisOutcome {
    pub wb: Workbook,
    pub calc: gridcraft_engine::calc::Calc,
    pub result: serde_json::Value,
    pub command: &'static str,
    pub params: serde_json::Value,
}

/// The recalculation outcome: the recomputed workbook, the fresh `Calc` to install beside it,
/// and the formula count (for the status text).
pub struct CalcOutcome {
    pub wb: Workbook,
    pub calc: gridcraft_engine::calc::Calc,
    pub cells: usize,
}

enum JobMessage {
    Progress(f64, String),
    Done(Box<JobResult>),
}

/// A job in flight: its kind (who applies what), title, latest progress, and the cancel flag.
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub title: String,
    /// 0..1 (nudged below 1.0 until the outcome arrives, so the bar never lies "100 %").
    pub progress: f64,
    pub status: String,
    pub cancel: Arc<AtomicBool>,
    /// Cooperative hooks of a calculation/analysis job: cancelling the job also stops the work
    /// in progress (not just discards the result).
    hooks: Option<Arc<CalcHooks>>,
    rx: Receiver<JobMessage>,
}

impl Job {
    /// Stops the work in progress: the flag plus the cooperative hooks (a running calculation or
    /// analysis stops at its next check instead of only being discarded).
    fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(h) = &self.hooks {
            h.cancel();
        }
    }
}

/// One running (or slightly finished) job per document-time operation. Read jobs (an open) can
/// coexist; a document has at most one save and one recalculation at a time — a repeated F9
/// cancels the previous calculation job (`SheetApp::run_command`).
///
/// [`SheetApp::run_command`]: crate::SheetApp::run_command
#[derive(Default)]
pub struct JobManager {
    next: JobId,
    jobs: Vec<Job>,
}

impl JobManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// The save of that document, if one is running (autosave never overlaps another save).
    pub fn is_saving(&self, doc_uid: u64) -> bool {
        self.jobs.iter().any(|j| matches!(j.kind, JobKind::Save { doc_uid: u, .. } if u == doc_uid))
    }
    /// The recalculation of that document, if one is running.
    pub fn is_calculating(&self, doc_uid: u64) -> bool {
        self.jobs.iter().any(|j| matches!(j.kind, JobKind::Calc { doc_uid: u, .. } if u == doc_uid))
    }
    /// Cancels a running job. The work still finishes what it is inside (a serialization pass);
    /// its outcome is reported cancelled and the interface applies nothing.
    pub fn cancel(&mut self, id: JobId) -> bool {
        match self.jobs.iter_mut().find(|j| j.id == id) {
            Some(j) => {
                j.stop();
                true
            }
            None => false,
        }
    }
    /// Cancels every save of a document (a failed save that keeps running is never wanted).
    pub fn cancel_saving(&mut self, doc_uid: u64) {
        for j in &self.jobs {
            if matches!(j.kind, JobKind::Save { doc_uid: u, .. } if u == doc_uid) {
                j.stop();
            }
        }
    }
    pub fn cancel_calculating(&mut self, doc_uid: u64) {
        for j in &self.jobs {
            if matches!(j.kind, JobKind::Calc { doc_uid: u, .. } if u == doc_uid) {
                j.stop();
            }
        }
    }
    pub fn cancel_analysis(&mut self, doc_uid: u64) {
        for j in &self.jobs {
            if matches!(j.kind, JobKind::Analysis { doc_uid: u } if u == doc_uid) {
                j.stop();
            }
        }
    }

    /// The running job of a document to show in the status bar, saves before recalculations.
    pub fn status_for(&self, doc_uid: u64) -> Option<&Job> {
        self.jobs.iter().find(|j| match j.kind {
            JobKind::Save { doc_uid: u, .. } => u == doc_uid,
            JobKind::Calc { doc_uid: u, .. } => u == doc_uid,
            JobKind::Analysis { doc_uid: u } => u == doc_uid,
        })
    }
    /// An analysis (Solver / Goal Seek) of that document, if one is running.
    pub fn is_analysis(&self, doc_uid: u64) -> bool {
        self.jobs.iter().any(|j| matches!(j.kind, JobKind::Analysis { doc_uid: u } if u == doc_uid))
    }

    fn register(&mut self, kind: JobKind, title: String, cancel: Arc<AtomicBool>, hooks: Option<Arc<CalcHooks>>, rx: Receiver<JobMessage>) -> JobId {
        let id = self.next;
        self.next += 1;
        self.jobs.push(Job { id, kind, title, progress: 0.0, status: String::new(), cancel, hooks, rx });
        id
    }

    /// Saves on a background thread: `plan` was prepared on the interface thread (`SavePlan::prepare`).
    /// Returns the job id even if the thread could not start — a disconnected channel reports the
    /// failure through [`JobManager::poll`], instead of the UI hanging on a half-registered job.
    pub fn spawn_save(&mut self, plan: SavePlan, doc_uid: u64, title: String, autosave: bool) -> JobId {
        let (tx, rx) = mpsc::channel::<JobMessage>();
        let cancel = Arc::new(AtomicBool::new(false));
        let kind = JobKind::Save { doc_uid, path: plan.path.clone() };
        let work = {
            let tx = tx.clone();
            let cancel = cancel.clone();
            move || {
                let _ = tx.send(JobMessage::Progress(0.02, String::new()));
                let outcome = if cancel.load(Ordering::Relaxed) {
                    Err(CANCELLED.to_string())
                } else {
                    let mut progress = |f: f64| {
                        let _ = tx.send(JobMessage::Progress(f, String::new()));
                    };
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plan.execute(Some(&mut progress)))) {
                        Ok(Ok(outcome)) => {
                            if cancel.load(Ordering::Relaxed) {
                                Err(CANCELLED.to_string())
                            } else {
                                Ok(outcome)
                            }
                        }
                        Ok(Err(e)) => Err(e.to_string()),
                        // SavePlan::execute keeps the workbook intact however it fails; a panic
                        // escaping it is reported like any other save error (never crash).
                        Err(p) => Err(compact_panic(p)),
                    }
                };
                let _ = tx.send(JobMessage::Done(Box::new(JobResult::Save { doc_uid, plan, outcome, autosave })));
            }
        };
        let spawned = detach(work);
        if !spawned {
            // Dropping the last sender makes poll report a failed job next frame.
            drop(tx);
        }
        self.register(kind, title, cancel, None, rx)
    }

    /// Recalculates a snapshot of `wb` (F9) on a background thread, reporting progress from the
    /// calculation itself. The caller keeps `wb` (the `Arc`) to guard the outcome's application,
    /// passes a clone of the live `calc` so a current dependency graph skips the full rebuild, and
    /// passes `seeds` so the background result matches the inline one.
    pub fn spawn_calc(
        &mut self,
        doc_uid: u64,
        wb: Arc<Workbook>,
        calc: gridcraft_engine::calc::Calc,
        full: bool,
        sheet: usize,
        seeds: gridcraft_engine::calc::CalcSeeds,
        title: String,
    ) -> JobId {
        let (tx, rx) = mpsc::channel::<JobMessage>();
        let cancel = Arc::new(AtomicBool::new(false));
        let kind = JobKind::Calc { doc_uid, full };
        let hooks = Arc::new(CalcHooks::new());
        {
            let tx = tx.clone();
            hooks.set_progress(Box::new(move |done, total| {
                let done = done.min(total.max(1));
                let frac = if total <= 1 { 0.0 } else { ((done as f64 / total as f64) * 0.9).min(0.9) };
                let _ = tx.send(JobMessage::Progress(frac, format!("{done} / {total}")));
            }));
        }
        let work = {
            let tx = tx.clone();
            let cancel = cancel.clone();
            let hooks = hooks.clone();
            move || {
                let outcome = if cancel.load(Ordering::Relaxed) {
                    Err(CANCELLED.to_string())
                } else {
                    let sheet = (!full).then_some(sheet);
                    let hooks = Some(hooks);
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gridcraft_engine::recalc_document(&wb, calc, sheet, hooks, seeds)))
                    {
                        // A cancelled run stops halfway: its snapshot is never applied.
                        Ok(_) if cancel.load(Ordering::Relaxed) => Err(CANCELLED.to_string()),
                        Ok((wb, calc, cells)) => Ok(CalcOutcome { wb, calc, cells }),
                        Err(p) => Err(compact_panic(p)),
                    }
                };
                let _ = tx.send(JobMessage::Done(Box::new(JobResult::Calc { doc_uid, wb, full, outcome: Box::new(outcome) })));
            }
        };
        let spawned = detach(work);
        if !spawned {
            drop(tx);
        }
        self.register(kind, title, cancel, Some(hooks), rx)
    }

    /// Runs Solver / Goal Seek on a snapshot (Fase 3): the analysis works on a private session on
    /// its own thread, reporting progress from its iterations and stopping when cancelled. The
    /// caller keeps `wb` (the `Arc`) to guard the outcome's application.
    pub fn spawn_analysis(
        &mut self,
        doc_uid: u64,
        wb: Arc<Workbook>,
        calc: gridcraft_engine::calc::Calc,
        command: &'static str,
        params: serde_json::Value,
        solver: bool,
        title: String,
    ) -> JobId {
        let (tx, rx) = mpsc::channel::<JobMessage>();
        let cancel = Arc::new(AtomicBool::new(false));
        let kind = JobKind::Analysis { doc_uid };
        let hooks = Arc::new(CalcHooks::new());
        {
            let tx = tx.clone();
            hooks.set_progress(Box::new(move |done, total| {
                let done = done.min(total.max(1));
                let frac = if total <= 1 { 0.0 } else { ((done as f64 / total as f64) * 0.9).min(0.9) };
                let _ = tx.send(JobMessage::Progress(frac, format!("{done} / {total}")));
            }));
        }
        let work = {
            let tx = tx.clone();
            let hooks = hooks.clone();
            move || {
                let outcome = if hooks.cancelled() {
                    Err(CANCELLED.to_string())
                } else {
                    let analyzed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        gridcraft_engine::run_analysis(&wb, &calc, command, &params, Some(hooks.clone()))
                    }));
                    match analyzed {
                        // A cancelled run stops halfway: its snapshot is never applied.
                        Ok(Ok(_)) if hooks.cancelled() => Err(CANCELLED.to_string()),
                        Ok(Ok((wb, calc, result))) => Ok(AnalysisOutcome { wb, calc, result, command, params }),
                        Ok(Err(e)) => Err(e.to_string()),
                        Err(p) => Err(compact_panic(p)),
                    }
                };
                let _ = tx.send(JobMessage::Done(Box::new(JobResult::Analysis { doc_uid, snapshot: wb, solver, outcome: Box::new(outcome) })));
            }
        };
        let spawned = detach(work);
        if !spawned {
            drop(tx);
        }
        self.register(kind, title, cancel, Some(hooks), rx)
    }

    /// Drains progress from every job each frame and hands back the finished ones, in the order
    /// they completed. Progress repaints (`request_repaint`) so the bar animates while jobs run.
    pub fn poll(&mut self, ctx: &Context) -> Vec<JobResult> {
        let mut finished: Vec<JobResult> = Vec::new();
        let mut drop_ids: Vec<JobId> = Vec::new();
        for job in &mut self.jobs {
            ctx.request_repaint();
            loop {
                match job.rx.try_recv() {
                    Ok(JobMessage::Progress(f, status)) => {
                        job.progress = f.clamp(0.0, 1.0);
                        job.status = status;
                    }
                    Ok(JobMessage::Done(result)) => {
                        drop_ids.push(job.id);
                        finished.push(*result);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    // A thread that never reported (spawn failure, abrupt death) is a failure,
                    // not a job that hangs the UI forever.
                    Err(TryRecvError::Disconnected) => {
                        drop_ids.push(job.id);
                        let kind = job.kind.clone();
                        finished.push(match kind {
                            JobKind::Save { doc_uid, .. } => JobResult::Save {
                                doc_uid,
                                plan: stub_plan(),
                                outcome: Err("the save job stopped unexpectedly".into()),
                                autosave: false,
                            },
                            JobKind::Calc { doc_uid, full } => JobResult::Calc {
                                doc_uid,
                                wb: Arc::new(Workbook::new()),
                                full,
                                outcome: Box::new(Err("the calculation job stopped unexpectedly".into())),
                            },
                            JobKind::Analysis { doc_uid } => JobResult::Analysis {
                                doc_uid,
                                snapshot: Arc::new(Workbook::new()),
                                solver: false,
                                outcome: Box::new(Err("the analysis job stopped unexpectedly".into())),
                            },
                        });
                        break;
                    }
                }
            }
        }
        if !drop_ids.is_empty() {
            self.jobs.retain(|j| !drop_ids.contains(&j.id));
        }
        finished
    }
}

/// A SavePlan stand-in for a poll-reported dead job: never executed, only carried.
fn stub_plan() -> SavePlan {
    SavePlan { wb: Arc::new(Workbook::new()), path: String::new(), kind: None, compression: 6 }
}

/// Runs `work` on a thread; `false` when the platform would not start one. Web: no threads —
/// the work runs now (slices-per-frame become possible with the same call shape later).
fn detach(work: impl FnOnce() + Send + 'static) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new().name("gridcraft-job".into()).spawn(work).is_ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        work();
        true
    }
}

fn compact_panic(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::Session;

    fn ctx() -> Context {
        Context::default()
    }

    fn session_with_book() -> (Session, u64) {
        let mut session = Session::new();
        session.new_workbook();
        let uid = session.active().map(|d| d.uid).unwrap_or_default();
        (session, uid)
    }

    fn plan_for(session: &Session, path: &str) -> SavePlan {
        let d = session.doc().map_err(|e| e.to_string()).unwrap();
        SavePlan::prepare(d, path.to_string())
    }

    /// Polls until a job reports, giving the worker thread time to run. A clock-free bounded
    /// wait (the disallowed-clock lint bans `Instant`/`SystemTime` here; the scheduler decides).
    fn wait_for_job(jobs: &mut JobManager, ctx: &Context) -> Vec<JobResult> {
        for _ in 0..5000 {
            let done = jobs.poll(ctx);
            if !done.is_empty() {
                return done;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Vec::new()
    }

    #[test]
    fn a_job_completes_on_its_own_and_poll_hands_back_the_outcome() {
        let mut jobs = JobManager::new();
        let dir = std::env::temp_dir().join("gridcraft-jobs-test-complete");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx").to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);
        let (session, uid) = session_with_book();
        let _id = jobs.spawn_save(plan_for(&session, &path), uid, "Saving Book.xlsx".into(), false);
        assert_eq!(jobs.jobs().len(), 1, "the job is registered");
        // The work runs on its own thread: the interface thread only polls.
        let ctx = ctx();
        let done = wait_for_job(&mut jobs, &ctx);
        assert!(!done.is_empty(), "the save job completed");
        assert!(std::path::Path::new(&path).exists(), "the save job wrote the file");
        match &done[0] {
            JobResult::Save { doc_uid, outcome, .. } => {
                assert_eq!(*doc_uid, uid);
                assert!(outcome.as_ref().map(|o| o.path == path).unwrap_or(false), "outcome: {outcome:?}");
            }
            JobResult::Calc { .. } | JobResult::Analysis { .. } => panic!("unexpected job kind"),
        }
        assert!(jobs.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_cancelled_save_writes_nothing_and_reports_cancelled() {
        let mut jobs = JobManager::new();
        let dir = std::env::temp_dir().join("gridcraft-jobs-test-cancel");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx").to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);
        let (session, uid) = session_with_book();
        let id = jobs.spawn_save(plan_for(&session, &path), uid, "Saving Book.xlsx".into(), false);
        assert!(jobs.cancel(id), "the job is there to cancel");
        let ctx = ctx();
        let done = wait_for_job(&mut jobs, &ctx);
        assert_eq!(done.len(), 1);
        match &done[0] {
            JobResult::Save { outcome, .. } => assert_eq!(outcome.as_ref().err().map(|e| e.as_str()), Some(CANCELLED)),
            JobResult::Calc { .. } | JobResult::Analysis { .. } => panic!("unexpected job kind"),
        }
        assert!(!std::path::Path::new(&path).exists(), "cancellation leaves the workbook file alone");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_finished_job_reports_back_the_original_document_uid() {
        // The apply step belongs to the interface; here we prove the outcome survives with the
        // original uid so `SheetApp::apply_job` can find the document by uid (or drop the result
        // when the user closed it meanwhile).
        let mut jobs = JobManager::new();
        let (session, uid) = session_with_book();
        let _id = jobs.spawn_save(plan_for(&session, "target/never-written.xlsx"), uid, "Saving".into(), false);
        let ctx = ctx();
        let mut saw_uid = None;
        for r in wait_for_job(&mut jobs, &ctx) {
            if let JobResult::Save { doc_uid, .. } = r {
                saw_uid = Some(doc_uid);
            }
        }
        assert_eq!(saw_uid, Some(uid));
    }

    #[test]
    fn an_edit_replaces_the_workbook_snapshot_a_calculation_guards_on() {
        // The guard lives in the interface (`apply_job` pointer check); this pins the data the
        // guard compares: an edit swaps the document's workbook `Arc`, so a job holding the old
        // `Arc` is detectably stale.
        let mut session = Session::new();
        session.new_workbook();
        let before = session.active().map(|d| d.wb.clone());
        session.run("cell.set", serde_json::json!({"cell": "A1", "input": "1"})).unwrap();
        let after = session.active().map(|d| d.wb.clone());
        assert!(!Arc::ptr_eq(before.as_ref().unwrap_or(after.as_ref().unwrap()), after.as_ref().unwrap()), "an edit replaces the workbook Arc");
    }
}

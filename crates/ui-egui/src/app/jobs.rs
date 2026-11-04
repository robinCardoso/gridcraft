use super::{Json, SheetApp, clean_error, json, now_ms};
use crate::jobs;

/// A save job's title: the file the job writes (the interface translates "Saving…"; the title
/// keeps the target name for tooltips and logs).
pub fn save_job_title(path: &str) -> String {
    let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
    format!("Saving {name}")
}

impl SheetApp {
    /// CSV/HTML export of a big workbook as a background job (docs/plano-responsividade.md,
    /// Fase 2: "Exportar… como jobs"). An export is a *copy*: the finished job applies nothing to
    /// the document, exactly like the inline command (`apply_job` leaves every non-workbook format
    /// alone). Exporting another sheet (`sheet` param) keeps the inline path: it clones the sheet
    /// in place and is the rare case.
    pub(super) fn export_job_attempt(&mut self, p: &Json, ext: &str) -> Option<Result<Json, String>> {
        if p.get("sheet").is_some() {
            return None;
        }
        let d = self.session.active()?;
        let path = p.get("path").and_then(Json::as_str)?.to_string();
        let cells: usize = d.wb.sheets.iter().map(|sh| sh.cells.len()).sum();
        if cells < self.job_thresholds.save_cells {
            return None;
        }
        // The engine appends the extension when the path has none; keep the same target.
        let target = if path.ends_with(&format!(".{ext}")) { path } else { format!("{path}.{ext}") };
        let kind = gridcraft_engine::io::FileKind::from_path(&target);
        let plan = gridcraft_engine::cmd::file::SavePlan { wb: d.wb.clone(), path: target.clone(), kind, compression: 6 };
        self.jobs.spawn_save(plan, d.uid, save_job_title(&target), false);
        Some(Ok(json!({"path": target, "pending": true})))
    }

    /// Defers a `file.close` while a save of the same document is in flight; `None` lets the
    /// engine close right away, as always.
    pub(super) fn close_job_deferral(&mut self) -> Option<Result<Json, String>> {
        let uid = self.session.active()?.uid;
        if self.jobs.is_saving(uid) || self.pending_save_uid == Some(uid) {
            self.pending_close_uid = Some(uid);
            return Some(Ok(json!({"closed": false, "pending": true})));
        }
        None
    }

    /// Completes a `file.close` that waited for a running save (Fase 2: "salvar e fechar"). It
    /// closes only when the document is still the active one and the save really cleaned it: a
    /// failed save leaves the document open (its error box is already up).
    pub(super) fn finish_pending_close(&mut self, uid: u64) {
        if self.pending_close_uid != Some(uid) {
            return;
        }
        self.pending_close_uid = None;
        let Some(d) = self.session.active() else { return };
        if d.uid != uid || d.is_dirty() {
            return;
        }
        let _ = self.session.run("file.close", json!({"force": true}));
        self.after_engine();
    }

    /// The save as a background job when the document is big enough for one to matter. `None`
    /// means "run the engine command inline as before". `fast` (the AutoSave) trades file size
    /// for time with deflate level 1 (docs/plano-responsividade.md, Fase 2).
    pub(super) fn save_job_attempt(&mut self, p: &Json, autosave: bool, fast: bool) -> Option<Result<Json, String>> {
        let d = self.session.active()?;
        let path = p.get("path").and_then(Json::as_str).map(str::to_string).or_else(|| d.path.clone())?;
        // PDFs go through the print pipeline and databases through their own saver, both inside
        // the engine command; import-only extensions (and unknown ones) run inline too.
        if path.to_ascii_lowercase().ends_with(".pdf") || gridcraft_engine::dbfile::is_db_path(&path) {
            return None;
        }
        match gridcraft_engine::io::FileKind::from_path(&path) {
            Some(
                gridcraft_engine::io::FileKind::Xlsx
                | gridcraft_engine::io::FileKind::Json
                | gridcraft_engine::io::FileKind::Csv
                | gridcraft_engine::io::FileKind::Tsv
                | gridcraft_engine::io::FileKind::Html,
            ) => {}
            Some(_) | None => return None,
        }
        let uid = d.uid;
        let cells: usize = d.wb.sheets.iter().map(|sh| sh.cells.len()).sum();
        if cells < self.job_thresholds.save_cells {
            return None;
        }
        if self.jobs.is_saving(uid) {
            // One save job per document (docs/plano-responsividade.md): a manual save while one
            // runs queues a follow-up that applies the still-dirty document afterwards.
            self.pending_save_uid = Some(uid);
            return Some(Ok(json!({"path": path, "pending": true})));
        }
        let mut plan = gridcraft_engine::cmd::file::SavePlan::prepare(d, path.clone());
        if fast {
            plan.compression = 1;
        }
        let title = save_job_title(&path);
        self.jobs.spawn_save(plan, uid, title, autosave);
        Some(Ok(json!({"path": path, "pending": true})))
    }

    /// F9 / Shift+F9 as a background calculation job when the workbook is big enough for one to
    /// matter. A repeated F9 cancels the previous calculation job (its result is discarded) and
    /// requeues on this snapshot — the plan's "cancel a previous grid and requeue". `force`
    /// bypasses the size gate: it requeues a recalculation the engine already owes
    /// (`recalc_pending`, Fase 3).
    pub(super) fn calc_job_attempt(&mut self, full: bool, force: bool) -> Option<Result<Json, String>> {
        let d = self.session.active()?;
        if !force && d.calc.graph.len() < self.job_thresholds.calc_formulas {
            return None;
        }
        let uid = d.uid;
        if self.jobs.is_calculating(uid) {
            self.jobs.cancel_calculating(uid);
        }
        let seeds = d.calc.seeds();
        let calc = d.calc.clone();
        let active_sheet = d.wb.active_sheet();
        let wb = d.wb.clone();
        let cascade = if force { d.recalc_cascade.clone() } else { None };
        let uncomputed_only =
            force && cascade.as_ref().map_or(true, Vec::is_empty) && d.calc.graph_is_current();
        let title = format!("Calculating {}", d.display_title());
        self.jobs.spawn_calc(uid, wb, calc, full, active_sheet, seeds, cascade, uncomputed_only, title);
        Some(Ok(json!({"pending": true})))
    }

    /// Solver / Goal Seek as a background job on big workbooks (docs/plano-responsividade.md,
    /// Fase 3): the iteration works on a snapshot through a private session, reports progress and
    /// stops on cancel. Small workbooks keep the inline path (fast, and the synchronous contract
    /// the tests and the control channel rely on).
    pub(super) fn analysis_job_attempt(&mut self, p: &Json, solver: bool) -> Option<Result<Json, String>> {
        let d = self.session.active()?;
        if d.calc.graph.len() < self.job_thresholds.calc_formulas {
            return None;
        }
        if solver {
            let has_objective =
                ["objective", "setObjective", "set"].iter().any(|k| p.get(*k).and_then(Json::as_str).is_some_and(|t| !t.trim().is_empty()));
            let saved = p.get("saved").and_then(Json::as_bool) == Some(true)
                && gridcraft_engine::cmd::solver::saved_model(&d.wb, d.wb.active_sheet()).is_some();
            if !has_objective && !saved {
                // Opening the Solver (no objective yet) is the dialog: keep it inline.
                return None;
            }
        }
        let uid = d.uid;
        if self.jobs.is_analysis(uid) {
            // The newest request wins: the previous snapshot is stale.
            self.jobs.cancel_analysis(uid);
        }
        let wb = d.wb.clone();
        let calc = d.calc.clone();
        let command = if solver { "data.solver" } else { "data.goalSeek" };
        let title = if solver { "Solver".to_string() } else { "Goal Seek".to_string() };
        self.jobs.spawn_analysis(uid, wb, calc, command, p.clone(), solver, title);
        Some(Ok(json!({"pending": true})))
    }

    /// Applies a finished job (docs/plano-responsividade.md §3.3). Guarded:
    /// a save is recorded against the document it wrote (found by uid — maybe closed meanwhile);
    /// a calculation is applied only while the workbook is still exactly the snapshot the job
    /// took, else it is discarded (an F9 made meanwhile can re-run on the current workbook).
    /// An autosave's errors log instead of a message box (the debounce retries).
    pub fn apply_job(&mut self, r: jobs::JobResult) {
        match r {
            jobs::JobResult::Save { doc_uid, plan, outcome, autosave } => match outcome {
                Ok(o) => {
                    if let Some(d) = self.session.doc_by_uid_mut(doc_uid) {
                        // What the save genuinely wrote: that snapshot on disk, not the workbook
                        // as it may be now. Only a full-fidelity format clears the dirty flag
                        // and becomes the document's file.
                        if matches!(plan.kind, Some(gridcraft_engine::io::FileKind::Xlsx | gridcraft_engine::io::FileKind::Json)) {
                            d.saved = plan.wb;
                            d.path = Some(o.path.clone());
                        }
                    }
                    log::info!("job: saved {} ({} KB)", o.path, o.bytes / 1024);
                    self.maybe_spawn_pending_save(doc_uid);
                    self.finish_pending_close(doc_uid);
                }
                Err(e) if e == jobs::CANCELLED => self.maybe_spawn_pending_save(doc_uid),
                Err(e) => {
                    if autosave {
                        log::warn!("AutoSave failed: {e}");
                    } else if self.session.doc_by_uid_mut(doc_uid).is_some() {
                        self.message = Some(("GridCraft".into(), clean_error(&e)));
                    }
                    self.maybe_spawn_pending_save(doc_uid);
                    self.finish_pending_close(doc_uid);
                }
            },
            jobs::JobResult::Calc { doc_uid, wb, full: _, outcome } => match *outcome {
                Ok(o) => {
                    if let Some(d) = self.session.doc_by_uid_mut(doc_uid)
                        && std::sync::Arc::ptr_eq(&d.wb, &wb)
                    {
                        d.wb = std::sync::Arc::new(o.wb);
                        d.calc = o.calc;
                        // Values changed under the same undo revision as the edit that queued the
                        // job: bump so grid text/CF caches (keyed by revision) drop stale
                        // `Text("")` paint from before the worker finished — otherwise body rows
                        // of a table stay blank on screen while expanded Empty rows paint fine.
                        d.revision = d.revision.saturating_add(1);
                        // The snapshot is fully recomputed: any owed recalculation is paid.
                        d.recalc_pending = false;
                        d.recalc_cascade = None;
                        self.calc_status = Some((1.0, o.cells, now_ms()));
                    } else {
                        log::info!("job: calculation result discarded (the workbook changed during it)");
                    }
                }
                Err(e) if e == jobs::CANCELLED => {}
                Err(e) => {
                    // Clear the debt so a failing calculation never requeues forever.
                    if let Some(d) = self.session.doc_by_uid_mut(doc_uid) {
                        d.recalc_pending = false;
                    }
                    self.message = Some(("GridCraft".into(), clean_error(&e)));
                }
            },
            jobs::JobResult::Analysis { doc_uid, snapshot, solver, outcome } => match *outcome {
                Ok(o) => {
                    let current = self.session.doc_by_uid_mut(doc_uid).is_some_and(|d| std::sync::Arc::ptr_eq(&d.wb, &snapshot));
                    if !current {
                        // An edit during the solve: its snapshot is stale, so nothing is written.
                        self.message =
                            Some(("GridCraft".into(), "The workbook changed during the analysis, so its result was discarded. Run it again.".into()));
                    } else {
                        let r = o.result.clone();
                        let _ = self.session.apply_analysis(doc_uid, o.wb, o.calc, o.command, &o.params);
                        if solver {
                            self.open_dialog("solverResults", r);
                        } else {
                            self.toast = Some((format!("Goal Seeking found a solution: {}", r["value"]), now_ms()));
                        }
                    }
                }
                Err(e) if e == jobs::CANCELLED => {}
                Err(e) => self.message = Some(("GridCraft".into(), clean_error(&e))),
            },
        }
    }

    /// A manual save requested while the document's save job was still running: once it ends and
    /// the document is still dirty (xlsx, still has its file), save again — now.
    pub(super) fn maybe_spawn_pending_save(&mut self, finished_uid: u64) {
        if self.pending_save_uid != Some(finished_uid) {
            return;
        }
        self.pending_save_uid = None;
        let Some(d) = self.session.doc_by_uid_mut(finished_uid) else { return };
        let Some(path) = d.path.clone() else { return };
        if !d.is_dirty() {
            return;
        }
        if !matches!(
            gridcraft_engine::io::FileKind::from_path(&path),
            Some(gridcraft_engine::io::FileKind::Xlsx | gridcraft_engine::io::FileKind::Json)
        ) {
            return;
        }
        let plan = gridcraft_engine::cmd::file::SavePlan::prepare(d, path.clone());
        let title = save_job_title(&path);
        self.jobs.spawn_save(plan, finished_uid, title, false);
    }
}

//! The GridCraft engine façade.
//!
//! Every user-visible action is a command with a stable id (`cell.set`, `home.bold`,
//! `data.sortAscending`…) and JSON parameters. The egui UI, the CLI, the control channel and MCP
//! all go through [`Session::execute`], so every action is scriptable and journaled.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod catalog;
pub mod cf;
mod clip_html;
pub mod cmd;
pub mod dbfile;
pub mod display;
pub mod fill;
pub mod io;
pub mod links;
pub mod outline;
pub mod paper;
pub mod pivot;
pub mod pivot_calc;
pub mod sample;
pub mod selection;
pub mod tables;

use std::sync::Arc;

use gridcraft_calc::Calc;
use gridcraft_core::{CellRef, RangeRef};
use gridcraft_model::{Workbook, WorkbookDiff};
use serde::Serialize;
use serde_json::Value as Json;

pub use cmd::file::load_document;
pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command, register_test_command};
pub use gridcraft_calc as calc;
pub use gridcraft_core as core;
pub use gridcraft_formula as formula;
pub use gridcraft_model as model;
pub use gridcraft_numfmt as numfmt;
pub use selection::Selection;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no workbook open")]
    NoDocument,
    #[error("{0}")]
    Other(String),
    #[error("internal error in `{0}` (the workbook was kept as it was): {1}")]
    Internal(String, String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// How an undo/redo step remembers the workbook (Fase 5 — docs/plano-responsividade.md).
///
/// Edits that only touch cell content and/or sheet metadata (comments, merges, dims, …) keep a
/// compact reverse/forward [`WorkbookDiff`]; workbook-level or structural changes keep a full COW
/// snapshot.
#[derive(Clone, Debug)]
pub enum HistoryState {
    Snapshot(Arc<Workbook>),
    /// `reverse` restores the pre-edit workbook from the post-edit one; `forward` does the opposite.
    Diff {
        reverse: WorkbookDiff,
        forward: WorkbookDiff,
    },
}

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub label: String,
    pub state: HistoryState,
    pub selection: Selection,
}

/// Above this many content-changed cells, a full snapshot is cheaper / simpler than a huge patch
/// (large paste, fill, etc.). Band COW already shares unchanged cell data across snapshots.
const UNDO_DIFF_MAX_CELLS: usize = 10_000;

fn history_state(before: Arc<Workbook>, after: &Workbook) -> HistoryState {
    match before.history_patches(after) {
        Some((reverse, forward)) if !reverse.is_empty() && reverse.cell_count() <= UNDO_DIFF_MAX_CELLS => HistoryState::Diff { reverse, forward },
        _ => HistoryState::Snapshot(before),
    }
}

fn push_undo(d: &mut DocState, label: String, before: Arc<Workbook>, selection: Selection) {
    let state = history_state(before, &d.wb);
    d.undo.push(HistoryEntry { label, state, selection });
    if d.undo.len() > UNDO_LIMIT {
        d.undo.remove(0);
    }
    d.redo.clear();
    d.revision += 1;
}

/// Applies the step's `reverse` patch (or swaps in a snapshot). Redo entries are stored with
/// reverse/forward swapped so this same helper walks either stack.
///
/// Cell patches only restore edited cells (formula caches of dependents stay stale until
/// [`Calc::cells_changed`] runs here); that path also keeps the dependency graph current via
/// [`Calc::update_nodes`] / `apply_node_updates` (same as a live edit). Full snapshots already
/// hold the right cached values but replace the workbook wholesale — the graph is marked
/// invalid so undo/redo's [`Calc::ensure_graph`] rebuilds once. Meta-only diffs skip the calc
/// hook (topology of formulas unchanged).
pub(crate) fn apply_history_state(d: &mut DocState, state: &HistoryState) {
    match state {
        HistoryState::Snapshot(wb) => {
            d.wb = wb.clone();
            d.calc.invalidate_graph();
        }
        HistoryState::Diff { reverse, .. } => {
            let keys: Vec<(usize, CellRef)> = reverse.cells.sheets.iter().flat_map(|sh| sh.cells.iter().map(|&(c, _)| (sh.sheet, c))).collect();
            let wb = Arc::make_mut(&mut d.wb);
            wb.apply_workbook_diff(reverse);
            if !keys.is_empty() {
                d.calc.cells_changed(wb, &keys);
            }
        }
    }
}

pub(crate) fn invert_history_state(state: HistoryState, current_wb: &Arc<Workbook>) -> HistoryState {
    match state {
        HistoryState::Snapshot(_) => HistoryState::Snapshot(current_wb.clone()),
        HistoryState::Diff { reverse, forward } => HistoryState::Diff { reverse: forward, forward: reverse },
    }
}

/// The clipboard: a block of cells (with formulas and styles) or marching-ants reference.
#[derive(Clone, Debug)]
pub struct Clipboard {
    /// Source workbook snapshot, sheet and range.
    pub wb: Arc<Workbook>,
    pub sheet: usize,
    pub range: RangeRef,
    /// The range as copied, before whole rows/columns were trimmed to the used range
    /// (Insert Copied Cells inserts whole lines for a row or column copy).
    pub selected: RangeRef,
    pub cut: bool,
    /// Document uid of the source (a cut only moves within the same workbook).
    pub doc_uid: u64,
    /// Plain text (tab-separated) for the system clipboard.
    pub text: String,
    /// A multi-area copy: the areas as selected (for the marquee). `wb` then holds them slid
    /// together into one block at `range`. Empty for a single-range copy.
    pub areas: Vec<RangeRef>,
}

/// In-cell edit mode as shown in the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Mode {
    #[default]
    Ready,
    Enter,
    Edit,
    Point,
}

#[derive(Clone, Debug)]
pub struct DocState {
    pub wb: Arc<Workbook>,
    pub calc: Calc,
    pub selection: Selection,
    pub undo: Vec<HistoryEntry>,
    pub redo: Vec<HistoryEntry>,
    pub path: Option<String>,
    pub saved: Arc<Workbook>,
    pub revision: u64,
    pub uid: u64,
    pub title: String,
    /// Per-sheet selection memory.
    pub sheet_selections: Vec<Selection>,
    /// Allow Edit Ranges unlocked with their password in this session: (sheet name, title).
    pub unlocked_ranges: Vec<(String, String)>,
    /// The format `path`'s content was recognised as when its extension doesn't name one (an
    /// XLSX called `budget.dat`): Save writes it back in that format.
    pub content_kind: Option<io::FileKind>,
    /// The interface asked for structural commands to skip the synchronous full recalculation
    /// (docs/plano-responsividade.md, Fase 3): a background job recalculates and applies the
    /// result. Never set by the CLI or the tests, which keep the synchronous contract.
    pub defer_structural: bool,
    /// A recalculation is owed (`defer_structural` skipped one): every commit defers until a
    /// background job applies the fresh values.
    pub recalc_pending: bool,
}

static NEXT_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
const UNDO_LIMIT: usize = 100;

/// The undo label of an analysis command (the same text its inline command would push).
fn command_label(command: &str) -> String {
    match command {
        "data.solver" => "Solver",
        "data.goalSeek" => "Goal Seek",
        _ => command,
    }
    .to_string()
}

/// Fase 3 (docs/plano-responsividade.md): when the interface turned `defer_structural` on and an
/// edit's recalculation cascade would evaluate more formulas than this, the commit skips the
/// synchronous recalculation and a background job recomputes everything. Below it, edits stay
/// inline so typing keeps feeling instant.
pub const CASCADE_JOB_CELLS: usize = 5_000;

impl DocState {
    pub fn new(mut wb: Workbook, path: Option<String>, title: String) -> DocState {
        let mut calc = Calc::new();
        calc.recalc_on_load(&mut wb);
        let wb = Arc::new(wb);
        let sel = Selection::at(wb.active().map(|s| s.view_active).unwrap_or_default());
        DocState {
            saved: wb.clone(),
            wb,
            calc,
            selection: sel,
            undo: vec![],
            redo: vec![],
            path,
            revision: 1,
            uid: NEXT_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            title,
            sheet_selections: vec![],
            unlocked_ranges: vec![],
            content_kind: None,
            defer_structural: false,
            recalc_pending: false,
        }
    }
    pub fn is_dirty(&self) -> bool {
        !Arc::ptr_eq(&self.wb, &self.saved)
    }
    pub fn sheet_index(&self) -> usize {
        self.wb.active_sheet
    }
    /// Records the view state that is saved with the file (`sheetView topLeftCell`, the active
    /// cell): the first visible cell of the scrolling area per sheet index in `tops`, and the
    /// active sheet's active cell. View state never makes the workbook dirty.
    pub fn store_view(&mut self, tops: &[(usize, CellRef)]) {
        let active = self.selection.active;
        let si = self.wb.active_sheet;
        let changed = tops.iter().any(|(i, c)| self.wb.sheets.get(*i).is_some_and(|sh| sh.view_top_left != *c))
            || self.wb.sheets.get(si).is_some_and(|sh| sh.view_active != active);
        if !changed {
            return;
        }
        let dirty = self.is_dirty();
        if !dirty {
            self.saved = Arc::new(Workbook::new());
            if let Some(wb) = Arc::get_mut(&mut self.wb) {
                for (i, c) in tops {
                    if let Some(sh) = wb.sheets.get_mut(*i)
                        && sh.view_top_left != *c
                    {
                        if let Some(sh_mut) = Arc::get_mut(sh) {
                            sh_mut.view_top_left = *c;
                        } else {
                            Arc::make_mut(sh).view_top_left = *c;
                        }
                    }
                }
                if let Some(sh) = wb.sheets.get_mut(si)
                    && sh.view_active != active
                {
                    if let Some(sh_mut) = Arc::get_mut(sh) {
                        sh_mut.view_active = active;
                    } else {
                        Arc::make_mut(sh).view_active = active;
                    }
                }
            } else {
                let wb = Arc::make_mut(&mut self.wb);
                for (i, c) in tops {
                    if let Some(sh) = wb.sheets.get_mut(*i)
                        && sh.view_top_left != *c
                    {
                        Arc::make_mut(sh).view_top_left = *c;
                    }
                }
                if let Some(sh) = wb.sheets.get_mut(si)
                    && sh.view_active != active
                {
                    Arc::make_mut(sh).view_active = active;
                }
            }
            self.saved = self.wb.clone();
        } else {
            let wb = Arc::make_mut(&mut self.wb);
            for (i, c) in tops {
                if let Some(sh) = wb.sheets.get_mut(*i)
                    && sh.view_top_left != *c
                {
                    Arc::make_mut(sh).view_top_left = *c;
                }
            }
            if let Some(sh) = wb.sheets.get_mut(si)
                && sh.view_active != active
            {
                Arc::make_mut(sh).view_active = active;
            }
        }
    }
    pub fn display_title(&self) -> String {
        self.path
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.title.clone())
    }
}

/// Engine-side preferences.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// After pressing Enter, move the selection: "down", "right", "up", "left", "none".
    pub enter_direction: String,
    pub autocomplete: bool,
    pub user_name: String,
    pub default_font: String,
    pub default_font_size: f32,
    pub sheets_in_new_workbook: usize,
    pub r1c1: bool,
    /// Words added with Add to Dictionary.
    pub user_dictionary: Vec<String>,
    /// View › Show › Formula Bar (an application option in Excel, not saved in the file).
    pub formula_bar: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            enter_direction: "down".into(),
            autocomplete: true,
            user_name: "GridCraft User".into(),
            default_font: gridcraft_model::DEFAULT_FONT.into(),
            default_font_size: gridcraft_model::DEFAULT_FONT_SIZE,
            sheets_in_new_workbook: 1,
            r1c1: false,
            user_dictionary: vec![],
            formula_bar: true,
        }
    }
}

/// Requests from commands to the UI (open a dialog, show a message…).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum UiRequest {
    Dialog(String, Json),
    Message(String),
    /// Start editing the active cell (F2) with optional initial text.
    EditCell(Option<String>),
    OpenUrl(String),
}

#[derive(Default)]
pub struct Session {
    docs: Vec<DocState>,
    active: usize,
    pub clipboard: Option<Clipboard>,
    pub prefs: Prefs,
    pub mode: Mode,
    /// Commands run so far, without their passwords ([`cmd::journal_params`]).
    pub journal: Vec<(String, Json)>,
    pub ui_requests: Vec<UiRequest>,
    /// Format painter source style (sticky when double-clicked).
    pub format_painter: Option<(Arc<Workbook>, usize, RangeRef, bool)>,
    /// Find state (last search).
    pub last_find: Option<Json>,
    /// View › Show › Ruler is off (rulers in Page Layout view; an application option).
    pub hide_ruler: bool,
    /// Journal position where action recording started.
    pub recording: Option<usize>,
    /// Windows hidden with View › Hide.
    pub hidden_windows: Vec<usize>,
    /// Draw tab tool: "" (select), "pen" or "eraser"; pen colour and width.
    pub draw_tool: String,
    pub draw_color: String,
    pub draw_width: f32,
    /// Review › Show Comments: the Comments pane is open.
    pub comments_pane: bool,
    /// Review › Notes › Show All Notes: every note is displayed. An application display setting
    /// (like Excel's comment indicator option), so it's not saved in the file.
    pub show_all_notes: bool,
    /// Cooperation hooks for a background analysis command (`data.solver`, `data.goalSeek`): the
    /// interface sets them on the private session a job runs on, so the iteration reports
    /// progress and stops on cancel (docs/plano-responsividade.md, Fase 3). `None` in the CLI.
    pub analysis_hooks: Option<std::sync::Arc<gridcraft_calc::CalcHooks>>,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }
    pub fn documents(&self) -> &[DocState] {
        &self.docs
    }
    pub fn active_index(&self) -> usize {
        self.active
    }
    pub fn set_active(&mut self, i: usize) {
        if i < self.docs.len() {
            self.active = i;
        }
    }
    pub fn active(&self) -> Option<&DocState> {
        self.docs.get(self.active)
    }
    pub fn active_mut(&mut self) -> Option<&mut DocState> {
        self.docs.get_mut(self.active)
    }
    pub fn doc(&self) -> Result<&DocState> {
        self.active().ok_or(EngineError::NoDocument)
    }
    pub fn doc_mut(&mut self) -> Result<&mut DocState> {
        self.active_mut().ok_or(EngineError::NoDocument)
    }
    /// A document by uid. Background jobs apply their outcomes to the document their snapshot
    /// came from, even after the user switched windows meanwhile.
    pub fn doc_by_uid_mut(&mut self, uid: u64) -> Option<&mut DocState> {
        self.docs.iter_mut().find(|d| d.uid == uid)
    }
    /// Applies a background analysis outcome (Solver, Goal Seek) to a document as ONE undoable
    /// step: the private session's workbook — values, saved model and report sheets — and its
    /// calculation state replace the document's. Nothing is recalculated here (the snapshot was
    /// fully computed on the worker); the entry mirrors what the inline command would have pushed.
    pub fn apply_analysis(&mut self, uid: u64, wb: Workbook, calc: Calc, command: &str, params: &Json) -> Result<()> {
        let Some(i) = self.docs.iter().position(|d| d.uid == uid) else { return Ok(()) };
        let d = &mut self.docs[i];
        let before = d.wb.clone();
        let sel = d.selection.clone();
        d.wb = Arc::new(wb);
        d.calc = calc;
        d.recalc_pending = false;
        self.journal.push((command.to_string(), cmd::journal_params(params)));
        if self.journal.len() > 10_000 {
            self.journal.drain(..5_000);
        }
        if !Arc::ptr_eq(&d.wb, &before) {
            push_undo(d, command_label(command), before, sel);
        }
        Ok(())
    }
    pub fn add_document(&mut self, d: DocState) -> usize {
        self.docs.push(d);
        self.active = self.docs.len() - 1;
        self.active
    }
    pub fn close_document(&mut self, i: usize) {
        if i < self.docs.len() {
            self.docs.remove(i);
            if self.active >= self.docs.len() {
                self.active = self.docs.len().saturating_sub(1);
            }
        }
    }
    /// A new blank workbook titled `BookN`.
    pub fn new_workbook(&mut self) -> usize {
        let n = (1..).find(|n| !self.docs.iter().any(|d| d.title == format!("Book{n}") && d.path.is_none())).unwrap_or(1);
        let mut wb = Workbook::new();
        for _ in 1..self.prefs.sheets_in_new_workbook.clamp(1, 255) {
            let name = wb.next_sheet_name();
            wb.sheets.push(Arc::new(gridcraft_model::Sheet::new(name)));
        }
        self.add_document(DocState::new(wb, None, format!("Book{n}")))
    }

    /// Mounts an asynchronously or synchronously loaded document into the session.
    pub fn install_document(&mut self, doc: DocState, warnings: Vec<String>) -> Result<Json> {
        // Replace an untouched blank Book1 like Excel does.
        if self.documents().len() == 1
            && self
                .active()
                .is_some_and(|d| d.path.is_none() && !d.is_dirty() && d.undo.is_empty() && d.wb.sheets.iter().all(|sh| sh.cells.is_empty()))
        {
            self.close_document(0);
        }
        let i = self.add_document(doc);
        let shown: Vec<&str> = warnings.iter().map(String::as_str).filter(|w| *w != crate::io::DECRYPTED_NOTICE).collect();
        if !shown.is_empty() {
            let mut message = shown.iter().take(5).copied().collect::<Vec<_>>().join("\n\n");
            if shown.len() > 5 {
                message.push_str(&format!("\n\n{} additional import warnings.", shown.len() - 5));
            }
            self.ui_requests.push(crate::UiRequest::Message(message));
        }
        let d = self.doc()?;
        Ok(serde_json::json!({
            "index": i,
            "title": d.display_title(),
            "sheets": d.wb.sheets.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
            "warnings": warnings,
        }))
    }

    pub fn commands(&self) -> Vec<CommandInfo> {
        command_specs().iter().map(|c| c.info(self)).collect()
    }

    /// Runs a command by id. Undoable commands record a history step when the workbook changed.
    /// Panics inside commands are caught and reported; the workbook is restored.
    pub fn execute(&mut self, id: &str, params: Json) -> Result<Json> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        if let Err(why) = (spec.enabled)(self) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        let before = self.active().map(|d| (d.uid, d.wb.clone(), d.selection.clone()));
        let params = if params.is_null() { Json::Object(Default::default()) } else { params };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, &params)));
        let result = match result {
            Ok(r) => r,
            Err(p) => {
                let msg =
                    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
                // Restore the workbook as it was.
                if let Some((uid, wb, sel)) = &before
                    && let Some(d) = self.docs.iter_mut().find(|d| d.uid == *uid)
                {
                    d.wb = wb.clone();
                    d.selection = sel.clone();
                    d.calc.rebuild(&d.wb);
                }
                return Err(EngineError::Internal(id.to_string(), msg));
            }
        };
        if spec.journal && result.is_ok() {
            self.journal.push((id.to_string(), cmd::journal_params(&params)));
            if self.journal.len() > 10_000 {
                self.journal.drain(..5_000);
            }
        }
        if spec.undoable
            && let Some((uid, wb, sel)) = before
            && let Some(d) = self.docs.iter_mut().find(|d| d.uid == uid)
            && !Arc::ptr_eq(&d.wb, &wb)
        {
            push_undo(d, spec.label.trim_end_matches('…').to_string(), wb, sel);
        }
        result
    }

    pub fn run(&mut self, id: &str, params: Json) -> std::result::Result<Json, String> {
        self.execute(id, params).map_err(|e| e.to_string())
    }

    pub fn take_ui_requests(&mut self) -> Vec<UiRequest> {
        std::mem::take(&mut self.ui_requests)
    }

    /// Runs a command for a caller with no UI (MCP, CLI): queued UI requests are dropped, and a command that only
    /// queued a file dialog because it got no file (`file.open`, `file.saveAs`, `file.save` on a workbook with no
    /// file yet, `data.getData`) is an error instead of a silent success.
    pub fn execute_headless(&mut self, id: &str, params: Json) -> Result<Json> {
        let r = self.execute(id, params);
        let requests = self.take_ui_requests();
        let asked_for_file = requests.iter().any(|q| matches!(q, UiRequest::Dialog(name, _) if name == "open" || name == "saveAs"));
        if r.is_ok() && asked_for_file {
            let what = match id {
                "file.open" => "`path` or `base64`",
                "data.getData" | "data.fromTextCsv" => "`path` or `text`",
                _ => "`path`",
            };
            let msg = format!("missing {what}: programmatic calls never open dialogs");
            return Err(EngineError::BadParams { cmd: id.to_string(), msg });
        }
        r
    }
}

/// Runs a what-if analysis command (`data.solver`, `data.goalSeek`) on a snapshot, as a pure
/// function any thread can run in a background job (docs/plano-responsividade.md, Fase 3). A
/// private [`Session`] owns the clone, so the iteration never touches the live document; the
/// result carries the workbook (values, saved model, report sheets), its calculation state and
/// the command's own JSON result (the Solver Results dialog reads it).
pub fn run_analysis(
    wb: &Workbook,
    calc: &Calc,
    command: &str,
    params: &Json,
    hooks: Option<Arc<gridcraft_calc::CalcHooks>>,
) -> Result<(Workbook, Calc, Json)> {
    let mut doc = DocState::new((*wb).clone(), None, "Analysis".into());
    // The snapshot's graph is already settled; the clone above only paid for `recalc_on_load`.
    doc.calc = calc.clone();
    let mut s = Session::new();
    s.install_document(doc, Vec::new())?;
    s.analysis_hooks = hooks;
    let result = s.execute(command, params.clone())?;
    let d = s.doc()?;
    Ok(((*d.wb).clone(), d.calc.clone(), result))
}

/// Recalculates a snapshot of a workbook — what F9 does — as a pure function any thread can run
/// in a background job (docs/plano-responsividade.md, Fase 3). `calc` is typically a clone of the
/// live calculation: when its graph is current, [`Calc::ensure_graph`] skips the O(formulas)
/// rebuild; when structurally invalidated (or empty), it rebuilds. Returns the recalculated
/// workbook, the [`Calc`] to install beside it (hooks detached), and the number of formulas
/// evaluated. With `sheet`, only that sheet's uncomputed formulas run (Shift+F9); the graph is
/// still ensured for the whole workbook. `seeds` carries the volatile calculation state
/// (`rand()` stream, fixed "now") so the result matches the inline calculation.
#[must_use]
pub fn recalc_document(
    wb: &Workbook,
    mut calc: Calc,
    sheet: Option<usize>,
    hooks: Option<Arc<gridcraft_calc::CalcHooks>>,
    seeds: gridcraft_calc::CalcSeeds,
) -> (Workbook, Calc, usize) {
    let mut wb = (*wb).clone();
    calc.set_seeds(seeds);
    if let Some(hooks) = hooks {
        calc.set_hooks(hooks);
    }
    match sheet {
        Some(i) => {
            calc.ensure_graph(&wb);
            calc.recalc_sheet(&mut wb, i);
        }
        None => calc.recalc_all(&mut wb),
    }
    calc.hooks = None;
    let cells = calc.last_recalc_cells;
    (wb, calc, cells)
}

/// Convenience: the active workbook's active sheet.
pub fn active_sheet(s: &Session) -> Option<&gridcraft_model::Sheet> {
    s.active().and_then(|d| d.wb.active())
}

pub fn cell(r: u32, c: u32) -> CellRef {
    CellRef::new(r, c)
}

#[cfg(test)]
mod tests;

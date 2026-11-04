use super::{SaveAnswer, SheetApp, json};

impl SheetApp {
    /// Holds back a window close (the close button, Alt+F4, the OS quitting the app) while a
    /// workbook has unsaved changes, and asks about them first.
    pub(super) fn guard_window_close(&mut self, ctx: &egui::Context) {
        if self.native_windows || self.allow_close || !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        // A cell still being edited is part of the workbook, as for File › Close.
        if !self.commit_edit(0, 0, false, false) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        // The macro step editor asks about its own unsaved steps and debug run first; once it has
        // closed, the window close goes on.
        if self.dialog.as_ref().is_some_and(|d| d.name == "macroEditor") {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            crate::macro_editor::request_close(self, true);
            return;
        }
        if !self.session.documents().iter().any(|d| d.is_dirty()) {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        match self.dialog.as_mut() {
            // Already asking about a workbook (File › Close): answering it now goes on to quit.
            Some(d) if d.name() == "saveChanges" => {
                d.values.insert("quit".into(), json!(true));
            }
            _ => self.continue_quit(ctx),
        }
    }

    /// Asks about the next unsaved workbook before quitting, in its window, or closes the app
    /// once none is left.
    pub(crate) fn continue_quit(&mut self, ctx: &egui::Context) {
        let dirty = self.session.documents().iter().find(|d| d.is_dirty()).map(|d| d.uid);
        match dirty.and_then(|uid| self.session.doc_window(uid)) {
            Some(window) => {
                // Its window comes forward (the native window too: see `doc_windows`).
                self.enter_window(window);
                let title = self.session.active().map(|d| d.display_title()).unwrap_or_default();
                self.open_dialog("saveChanges", json!({"title": title, "quit": true}));
                self.bring_forward = true;
            }
            None => {
                self.allow_close = true;
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            }
        }
    }

    /// Answers the Save Changes question about the active workbook (its buttons). While quitting,
    /// the next unsaved workbook is asked about once this one is answered; Cancel stops the quit.
    /// Returns whether the question is settled: Save keeps it open when the cell being edited
    /// can't be entered.
    pub fn answer_save_changes(&mut self, answer: SaveAnswer, quitting: bool, ctx: &egui::Context) -> bool {
        match answer {
            SaveAnswer::Cancel => {}
            SaveAnswer::DontSave => {
                self.run_or_alert("file.close", json!({"force": true, "document": true}));
                if quitting {
                    self.continue_quit(ctx);
                }
            }
            SaveAnswer::Save => {
                // File operations include the cell still being edited, as File › Save does.
                if !self.commit_edit(0, 0, false, false) {
                    return false;
                }
                self.run_or_alert("file.save", json!({}));
                if self.session.active().is_some_and(|x| !x.is_dirty()) {
                    self.run_or_alert("file.close", json!({"force": true, "document": true}));
                    if quitting {
                        self.continue_quit(ctx);
                    }
                }
            }
        }
        true
    }

    /// Quit (the macOS app menu, ⌘Q): commits every window's cell draft as Enter would, lets the
    /// macro step editor ask about its unsaved steps, then asks about each unsaved workbook in its
    /// window; Cancel on any of them stops the quit. A draft its
    /// workbook refuses stops the quit too, with its window in front and the reason shown.
    pub fn request_quit(&mut self, ctx: &egui::Context) {
        while let Some(window) = self.parked_window_with_draft() {
            self.enter_window(window);
            if self.settle_drafts().is_err() {
                return;
            }
        }
        if self.settle_drafts().is_err() {
            return;
        }
        // The macro step editor asks about its own unsaved steps and debug run first, as for the
        // window's close button, in the window it is shown in; once it has closed, the quit goes
        // on, and Cancel stops it.
        if let Some(window) = self.window_with_dialog("macroEditor") {
            if self.ui_window() != Some(window) {
                self.enter_window(window);
            }
            crate::macro_editor::request_close(self, true);
            return;
        }
        if let Some(d) = self.dialog.as_mut().filter(|d| d.name() == "saveChanges") {
            // Already asking about a workbook (File › Close): answering it goes on to quit.
            d.values.insert("quit".into(), json!(true));
            return;
        }
        self.continue_quit(ctx);
    }
    /// Forced close after Save, or Don't Save (`discard`). Drops a heavy `DocState` off the UI
    /// thread when it would hitch the frame (Windows "Not responding" on LISTA-scale books).
    pub(super) fn force_close_active_document(&mut self, discard: bool) -> Result<serde_json::Value, String> {
        let Some(d) = self.session.active() else {
            return Err("no workbook open".into());
        };
        let uid = d.uid;
        // Save then Close: wait for the in-flight save (Fase 2). Don't Save cancels it instead.
        if !discard && (self.jobs.is_saving(uid) || self.pending_save_uid == Some(uid)) {
            self.pending_close_uid = Some(uid);
            return Ok(json!({"closed": false, "pending": true}));
        }
        self.jobs.cancel_calculating(uid);
        self.jobs.cancel_analysis(uid);
        if discard {
            self.jobs.cancel_saving(uid);
            if self.pending_save_uid == Some(uid) {
                self.pending_save_uid = None;
            }
        }
        if self.pending_close_uid == Some(uid) {
            self.pending_close_uid = None;
        }
        self.calc_status = None;
        self.selected_object = None;
        let i = self.session.active_index();
        let removed = self.session.close_document(i);
        Self::discard_document_off_ui_thread(removed);
        self.after_engine();
        Ok(json!({"closed": true}))
    }

    /// Frees a closed document. Small books drop inline; large ones (LISTA-scale) hitch Windows
    /// "Not responding" if `Drop` runs on the interface thread, so native builds hand them to a
    /// short-lived worker. Wasm has no threads — drop stays synchronous.
    pub(crate) fn discard_document_off_ui_thread(doc: Option<gridcraft_engine::DocState>) {
        let Some(doc) = doc else {
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cells: usize = doc.wb.sheets.iter().map(|sh| sh.cells.len()).sum();
            let heavy = doc.calc.graph.len() >= 50_000 || cells >= 50_000;
            if heavy {
                #[allow(clippy::disallowed_methods)]
                let _ = std::thread::Builder::new().name("gridcraft-doc-drop".into()).spawn(move || {
                    drop(doc);
                });
                return;
            }
        }
        drop(doc);
    }
}

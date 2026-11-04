use super::{Json, Notice, NoticeKind, OpenWindow, SheetApp, clean_error, json};

/// The file name to show for a path or a picked name (the loading modal's title).
pub fn file_name(path_or_name: &str) -> String {
    std::path::Path::new(path_or_name).file_name().and_then(|n| n.to_str()).unwrap_or("Book").to_string()
}

impl SheetApp {
    pub fn open_path(&mut self, path: &str) {
        self.open_path_async(path);
    }

    pub fn open_path_async(&mut self, path: &str) {
        self.backstage = None;
        if let Some(i) = self.session.documents().iter().position(|d| d.path.as_deref() == Some(path)) {
            self.session.set_active(i);
            self.after_open(Some(path), &serde_json::json!({"index": i, "alreadyOpen": true}));
            self.after_engine();
            return;
        }
        let title = file_name(path);
        self.open_document_async(json!({"path": path}), title, Some(path.to_string()), None, false);
    }

    pub fn open_bytes_async(&mut self, name: &str, base64: String, password: Option<String>) {
        let title = file_name(name);
        self.open_document_async(
            json!({"name": name, "base64": base64, "password": password}),
            title,
            None,
            Some(json!({"name": name, "base64": base64})),
            false,
        );
    }

    pub fn open_document_async(&mut self, params: Json, title: String, display: Option<String>, open_params: Option<Json>, retry: bool) {
        self.backstage = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let tx_status = tx.clone();
            let is_pt = self.ui.language == crate::i18n::Language::PtBr;
            let initial_status = if is_pt { "Lendo dados da planilha...".to_string() } else { "Reading spreadsheet data...".to_string() };
            let task_params = params.clone();
            let spawned = std::thread::Builder::new().name("gridcraft-loader".into()).spawn(move || {
                let res = gridcraft_engine::load_document(
                    &task_params,
                    Some(Box::new(move |msg| {
                        let _ = tx_status.send(crate::LoadTaskMessage::Status(msg.to_string()));
                    })),
                )
                .map_err(|e| e.to_string());
                let _ = tx.send(crate::LoadTaskMessage::Done(Box::new(res)));
            });
            if spawned.is_ok() {
                self.loading = Some(crate::LoadingTask { title, path: display.clone().unwrap_or_default(), status: initial_status, open_params, retry, rx });
                return;
            }
        }
        // wasm (no threads) or spawn failure: the synchronous path as before.
        match self.run("file.open", params) {
            Ok(r) => match display {
                Some(path) => self.after_open(Some(&path), &r),
                None => self.show_open_warnings(&r),
            },
            Err(e) if e.contains(gridcraft_engine::io::PASSWORD_REQUIRED) => {
                self.open_dialog("openPassword", open_params.unwrap_or_else(|| json!({"path": display.unwrap_or_default()})));
            }
            Err(e) => self.message = Some(("GridCraft".into(), clean_error(&e))),
        }
        self.after_engine();
    }

    pub(super) fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some(loading) = &mut self.loading else { return };
        ctx.request_repaint();
        let mut done = None;
        while let Ok(msg) = loading.rx.try_recv() {
            match msg {
                crate::LoadTaskMessage::Status(s) => loading.status = s,
                crate::LoadTaskMessage::Done(res) => {
                    done = Some(*res);
                    break;
                }
            }
        }
        if done.is_none() && matches!(loading.rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected)) {
            done = Some(Err("the open stopped unexpectedly".into()));
        }
        if let Some(res) = done {
            let path = loading.path.clone();
            let open_params = loading.open_params.clone();
            let _retry = loading.retry;
            self.loading = None;
            let _on_disk = open_params.is_none();
            match res {
                Ok((doc, warnings)) => match self.session.install_document(doc, warnings) {
                    Ok(r) => {
                        let path_ref = if path.is_empty() { None } else { Some(path.as_str()) };
                        self.after_open(path_ref, &r);
                    }
                    Err(e) => self.message = Some(("GridCraft".into(), clean_error(&e.to_string()))),
                },
                Err(e) if e.contains(gridcraft_engine::io::PASSWORD_REQUIRED) => {
                    let params = open_params.unwrap_or_else(|| json!({"path": path}));
                    self.open_dialog("openPassword", params);
                }
                Err(e) => {
                    let err = clean_error(&e);
                    self.message = Some(("GridCraft".into(), err));
                }
            }
            self.after_engine();
        }
    }

    /// What follows a successful `file.open`: the workbook shows (not the Backstage), the Recent
    /// list (for a file on disk) and the open warnings notice. Shared by File › Open, the
    /// password dialog and the control channel.
    pub fn after_open(&mut self, path: Option<&str>, r: &Json) {
        self.backstage = None;
        if let Some(path) = path {
            self.ui.recent.retain(|p| p != path);
            self.ui.recent.insert(0, path.to_string());
            self.ui.recent.truncate(20);
        }
        self.show_open_warnings(r);
    }

    /// Turns the `warnings` a `file.open` result carries into an info-bar notice, choosing the
    /// most important one (a preserved VBA project outranks, say, a skipped optional part).
    /// Kept deliberately generic: any open-time warning can reach the user this way.
    pub fn show_open_warnings(&mut self, r: &Json) {
        // The opened workbook's window shows them, however it was opened (a script may have run
        // `file.open` on the session directly).
        self.follow_active_window();
        let Some(list) = r.get("warnings").and_then(Json::as_array) else {
            self.notice = None;
            return;
        };
        let msgs: Vec<&str> = list.iter().filter_map(Json::as_str).collect();
        if msgs.is_empty() {
            // A clean open clears any notice left over from the previous document.
            self.notice = None;
            return;
        }
        let vba: Vec<&str> = msgs.iter().copied().filter(|m| m.to_lowercase().contains("macro") || m.contains("VBA")).collect();
        if !vba.is_empty() {
            // The reader says "dropped" when the VBA project was missing or unreadable; never promise to keep it then.
            let dropped = vba.iter().any(|m| m.contains("dropped"));
            let text = if dropped {
                "This workbook's VBA macros could not be loaded and were dropped, so they will not be in the saved file. Keep the original file."
            } else {
                "This workbook contains VBA macros. GridCraft keeps them when you save as .xlsm, but does not run them."
            };
            self.notice = Some(Notice { text: text.into(), kind: NoticeKind::Warning, action: Some(("What this means".into(), "macros".into())) });
        } else {
            let text = if msgs.len() == 1 {
                msgs[0].to_string()
            } else {
                format!("{} items in this file were handled differently. First: {}", msgs.len(), msgs[0])
            };
            self.notice = Some(Notice { text, kind: NoticeKind::Info, action: None });
        }
    }

    /// The open windows in order, for the Switch Windows lists.
    pub fn open_windows(&self) -> Vec<OpenWindow> {
        let active = self.session.active_window();
        self.session
            .windows()
            .iter()
            .filter_map(|w| {
                let d = self.session.documents().iter().find(|d| d.uid == w.doc_uid)?;
                let title = self.session.window_title(w.id)?;
                Some(OpenWindow { window: w.id, doc: d.uid, title, dirty: d.is_dirty(), active: active == Some(w.id) })
            })
            .collect()
    }

    /// Opens files that arrived asynchronously (web file picker, dropped files).
    pub(super) fn poll_async_opens(&mut self) {
        let arrived: Vec<(String, Vec<u8>)> = self
            .services
            .inbox
            .as_ref()
            .map(|i| std::mem::take(&mut *i.lock().unwrap_or_else(std::sync::PoisonError::into_inner)))
            .unwrap_or_default();
        for (name, bytes) in arrived {
            let b64 = gridcraft_engine::io::base64_encode(&bytes);
            match self.run("file.open", json!({"name": &name, "base64": &b64})) {
                // A file picked from File › Open (web) arrives here: show it, not the Backstage.
                Ok(r) => {
                    self.backstage = None;
                    self.show_open_warnings(&r);
                }
                Err(e) if e.contains(gridcraft_engine::io::PASSWORD_REQUIRED) => {
                    self.backstage = None;
                    self.open_dialog("openPassword", json!({"name": name, "base64": b64}));
                }
                Err(e) => self.message = Some(("GridCraft".into(), clean_error(&e))),
            }
            self.after_engine();
        }
    }

    /// AutoSave: save workbooks that have a file shortly after they change. This decision lives in
    /// `logic` so a hidden or logic-only frame still saves; the monotonic timestamp check keeps a
    /// forced hidden UI pass from saving the same change twice.
    pub(super) fn autosave(&mut self, ctx: &egui::Context) {
        let dirty_xlsx = self.session.active().is_some_and(|d| d.is_dirty() && d.path.as_deref().is_some_and(|p| p.ends_with(".xlsx")));
        let now = crate::monotonic_ms();
        let autosave_due =
            self.ui.autosave && self.editor.is_none() && self.text_box_editor.is_none() && now - self.grid.last_autosave > 2000.0 && dirty_xlsx;
        if autosave_due {
            self.grid.last_autosave = now;
            self.store_view_state();
            let _ = self.session.run("file.save", json!({}));
        } else if self.ui.autosave && dirty_xlsx {
            // Keep checking even when no input arrives while the window is hidden.
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }
    }

    /// Flip AutoSave (title-bar switch / Options ▸ Save). Persisted with the other UI
    /// preferences, so it stays as the user left it across restarts (#1901).
    pub fn toggle_autosave(&mut self) {
        self.ui.autosave = !self.ui.autosave;
    }
}

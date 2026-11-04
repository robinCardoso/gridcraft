use egui::{Stroke, vec2};

use super::show::ok_cancel;
use super::{Dialog, Json, Key, SheetApp, json};

pub(super) fn name_manager(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, open: &mut bool) {
    // Excel Name Manager: toolbar → list → "Refers to:" of the selection → Close in the footer.
    let lang = app.ui.language;
    let list = app.session.run("formulas.nameManager", json!({})).unwrap_or(Json::Null);
    let names = list.as_array().cloned().unwrap_or_default();
    if d.list_index >= names.len() {
        d.list_index = names.len().saturating_sub(1);
    }
    let selected = names.get(d.list_index).cloned();
    let field = |n: &Json, k: &str| n.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let open_edit = |app: &mut SheetApp, n: &Json| {
        app.open_dialog(
            "defineName",
            json!({
                "name": field(n, "name"),
                "scope": field(n, "scope"),
                "comment": field(n, "comment"),
                "refersTo": field(n, "refersTo"),
                "oldName": field(n, "name"),
                "oldScope": field(n, "scope"),
            }),
        );
    };

    ui.horizontal(|ui| {
        if ui.button(lang.tr("New…")).clicked() {
            app.open_dialog("defineName", json!({}));
        }
        if ui.add_enabled(selected.is_some(), egui::Button::new(lang.tr("Edit…"))).clicked()
            && let Some(n) = &selected
        {
            open_edit(app, n);
        }
        if ui.add_enabled(selected.is_some(), egui::Button::new(lang.tr("Delete"))).clicked()
            && let Some(n) = &selected
        {
            app.run_or_alert("formulas.deleteName", json!({"name": field(n, "name"), "scope": field(n, "scope")}));
        }
    });
    ui.add_space(6.0);
    ui.separator();

    let list_height = 240.0;
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, crate::theme::Tokens::get(ui.ctx()).separator))
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("name_manager_list")
                .max_height(list_height)
                .min_scrolled_height(list_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    egui::Grid::new("names")
                        .striped(true)
                        .num_columns(4)
                        .min_col_width(80.0)
                        .spacing(vec2(12.0, 4.0))
                        .show(ui, |ui| {
                            ui.strong(lang.tr("Name"));
                            ui.strong(lang.tr("Value"));
                            ui.strong(lang.tr("Refers To"));
                            ui.strong(lang.tr("Scope"));
                            ui.end_row();
                            if names.is_empty() {
                                ui.label(egui::RichText::new(lang.tr("No names defined")).italics().weak());
                                ui.end_row();
                            }
                            for (i, n) in names.iter().enumerate() {
                                let name = field(n, "name");
                                let value: String = n
                                    .get("value")
                                    .map(|v| v.to_string())
                                    .unwrap_or_default()
                                    .chars()
                                    .take(36)
                                    .collect();
                                let refers = field(n, "refersTo");
                                let scope = field(n, "scope");
                                let selected_row = i == d.list_index;
                                let mut pick = |ui: &mut egui::Ui, label: &str| {
                                    let resp = ui.selectable_label(selected_row, label);
                                    if resp.clicked() {
                                        d.list_index = i;
                                    }
                                    if resp.double_clicked() {
                                        d.list_index = i;
                                        open_edit(app, n);
                                    }
                                };
                                // Clicking any column selects the row (Excel list behaviour).
                                pick(ui, &name);
                                pick(ui, &value);
                                pick(ui, &refers);
                                pick(ui, &scope);
                                ui.end_row();
                            }
                        });
                });
        });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(lang.tr("Refers to:"));
        let mut refers = selected.as_ref().map(|n| field(n, "refersTo")).unwrap_or_default();
        // Preview only — Excel edits here with ✓/✗; we keep edits on Edit… / double-click.
        ui.add_enabled(
            false,
            egui::TextEdit::singleline(&mut refers).desired_width(ui.available_width().max(120.0)),
        );
    });
    if let Some(comment) = selected.as_ref().map(|n| field(n, "comment")).filter(|c| !c.is_empty()) {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(comment).small().weak());
    }

    // Keep Close on a single footer row. Bare `right_to_left(Align::Center)` expands to the
    // remaining window height and floats the button mid-panel (Excel keeps it bottom-right).
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(format!("  {}  ", lang.tr("Close"))).clicked() {
                *open = false;
            }
        });
    });
}

/// The names Paste Names offers in the active workbook.
pub(super) fn pasteable_names(app: &SheetApp) -> Vec<String> {
    app.session
        .active()
        .map(|doc| gridcraft_engine::cmd::formulas::pasteable_names(&doc.wb).into_iter().map(|(n, _)| n).collect())
        .unwrap_or_default()
}

/// Puts a defined name into the formula being edited, at the caret. Without an edit under way it
/// starts one in the active cell with `=` and the name.
pub(crate) fn insert_name(app: &mut SheetApp, name: &str) {
    match app.editor.as_mut() {
        Some(ed) => ed.insert_at_caret(name),
        None => app.begin_edit(Some(format!("={name}")), false),
    }
}

/// Paste Name (F3, Formulas › Use in Formula › Paste Names…): OK, or a double-click, inserts the
/// selected name (`formulas.pasteName` with `name`); Paste List writes every name and what it
/// refers to from the active cell.
pub(super) fn paste_name(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let names: Vec<String> =
        d.result.as_ref().and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).map(str::to_owned).collect()).unwrap_or_default();
    // The `name` value is the selection, so `ui.dialog.set` can choose one too.
    if let Some(i) = d.values.get("name").and_then(Json::as_str).and_then(|n| names.iter().position(|x| x == n)) {
        d.list_index = i;
    }
    let (down, up) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, Key::ArrowUp)));
    let last = names.len().saturating_sub(1);
    if down {
        d.list_index = (d.list_index + 1).min(last);
    }
    if up {
        d.list_index = d.list_index.saturating_sub(1);
    }
    d.list_index = d.list_index.min(last);
    ui.label(app.ui.language.tr("Paste name"));
    if names.is_empty() {
        ui.label(egui::RichText::new(app.ui.language.tr("No names defined")).italics());
    }
    // A workbook can hold thousands of names: only the rows in view are laid out.
    let row_height = ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
    let step = row_height + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show_rows(ui, row_height, names.len(), |ui, rows| {
        let top = ui.cursor().top() - rows.start as f32 * step;
        for (i, n) in names.iter().enumerate().skip(rows.start).take(rows.len()) {
            let r = ui.selectable_label(i == d.list_index, n);
            if r.clicked() {
                d.list_index = i;
            }
            if r.double_clicked() {
                *confirm = true;
            }
        }
        // The arrow keys can move the selection onto a row that isn't laid out.
        if down || up {
            let rect = egui::Rect::from_min_size(egui::pos2(ui.min_rect().left(), top + d.list_index as f32 * step), egui::vec2(1.0, row_height));
            if !ui.clip_rect().contains_rect(rect) {
                ui.scroll_to_rect(rect, Some(if down { egui::Align::Max } else { egui::Align::Min }));
            }
        }
    });
    ui.add_space(4.0);
    // Paste List is disabled while a cell is being edited.
    if ui.add_enabled(!names.is_empty() && app.editor.is_none(), egui::Button::new(app.ui.language.tr("Paste List"))).clicked() {
        app.run_or_alert("formulas.pasteName", json!({"list": true}));
        *open = false;
        return;
    }
    if let Some(n) = names.get(d.list_index) {
        d.values.insert("name".into(), json!(n));
    }
    ok_cancel(ui, confirm, open);
}

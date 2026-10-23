//! Data-validation list picker: type-to-filter while the dropdown is open.

use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::{SheetApp, grid};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    session
        .execute("data.validation", json!({"range": "B2", "type": "list", "formula1": "\"CE,DF,ES,GO,MA,MT,MS,SC,SP\""}))
        .unwrap();
    session.execute("selection.set", json!({"cell": "B2"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    h
}

fn open_list(h: &mut Harness) {
    let app = h.state();
    let sh = app.session.active().unwrap().wb.active().unwrap();
    let geo = grid::Geo::new(sh, app.grid.rect.unwrap(), app.view().scroll);
    let pos = grid::validation_arrow(sh, &geo, CellRef::parse("B2").unwrap()).unwrap().center();
    h.input_mut().events.push(Event::PointerMoved(pos));
    h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().grid.list_picker.is_some(), "the dropdown arrow opens the list");
}

fn key(h: &mut Harness, k: Key, m: Modifiers) {
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
    h.run_steps(2);
}

fn cell_text(h: &Harness) -> String {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::parse("B2").unwrap()).display()
}

#[test]
fn typing_filters_the_validation_list_and_enter_commits() {
    let mut h = harness();
    open_list(&mut h);
    assert!(h.query_by_label("CE").is_some());
    assert!(h.query_by_label("SC").is_some());

    h.input_mut().events.push(Event::Text("s".into()));
    h.run_steps(2);
    assert!(h.state().editor.is_none(), "typing must not start a cell edit while the list is open");
    assert!(h.state().grid.list_picker.is_some());
    assert!(h.query_by_label("SC").is_some(), "SC contains 's'");
    assert!(h.query_by_label("ES").is_some(), "ES contains 's'");
    assert!(h.query_by_label("MS").is_some(), "MS contains 's'");
    assert!(h.query_by_label("CE").is_none(), "CE does not contain 's'");
    assert!(h.query_by_label("DF").is_none());

    h.input_mut().events.push(Event::Text("c".into()));
    h.run_steps(2);
    assert!(h.query_by_label("SC").is_some(), "only SC matches 'sc'");
    assert!(h.query_by_label("ES").is_none());
    assert!(h.query_by_label("MS").is_none());

    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().grid.list_picker.is_none(), "Enter closes the list");
    assert_eq!(cell_text(&h), "SC");
}

#[test]
fn filter_is_case_insensitive_substring_and_empty_when_no_match() {
    let mut h = harness();
    open_list(&mut h);

    h.input_mut().events.push(Event::Text("Mt".into()));
    h.run_steps(2);
    assert!(h.query_by_label("MT").is_some());
    assert!(h.query_by_label("MA").is_none());

    h.input_mut().events.push(Event::Text("zzz".into()));
    h.run_steps(2);
    assert!(h.query_by_label("MT").is_none());
    assert!(h.state().grid.list_picker.is_some(), "no match leaves the list open and empty");
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().grid.list_picker.is_some(), "Enter does nothing with an empty match list");
    assert_eq!(cell_text(&h), "");
}

#[test]
fn click_still_commits_a_filtered_item() {
    let mut h = harness();
    open_list(&mut h);
    h.input_mut().events.push(Event::Text("g".into()));
    h.run_steps(2);
    assert!(h.query_by_label("GO").is_some());

    let go = h.get_by_label("GO").rect().center();
    h.input_mut().events.push(Event::PointerMoved(go));
    h.input_mut().events.push(Event::PointerButton { pos: go, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::PointerButton { pos: go, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().grid.list_picker.is_none());
    assert_eq!(cell_text(&h), "GO");
}

//! Find & Replace: Ctrl+L opens Find (Excel pt-BR), Ctrl+H opens it on the Replace tab, and
//! Find All reports matches with their formulas (docs/plano-responsividade.md).

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::{Services, SheetApp};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("range.setValues", json!({"range": "A1", "values": [["alpha"], ["beta"], ["alphabet"]]})).unwrap();
    session.execute("selection.set", json!({"cell": "A1"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Services::default()),
    );
    h.run_steps(4);
    h
}

/// A Ctrl/Cmd chord, as the platform sends it (`command` is Ctrl on Windows/Linux).
fn press(h: &mut Harness, key: Key) {
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::COMMAND });
    }
    h.run_steps(2);
}

#[test]
fn ctrl_l_opens_find_and_ctrl_h_opens_replace() {
    if cfg!(target_os = "macos") {
        // macOS keeps Excel's Cmd+L = Align Left and Ctrl+L = Define Name, so the pt-BR binding
        // is a Windows/Linux convention.
        return;
    }
    let mut h = harness();
    press(&mut h, Key::L);
    let d = h.state().dialog.as_ref().expect("Ctrl+L opens Find and Replace");
    assert_eq!(d.name, "find");
    assert_eq!(d.tab, "Find");
    h.state_mut().dialog = None;
    press(&mut h, Key::H);
    let d = h.state().dialog.as_ref().expect("Ctrl+H opens Find and Replace");
    assert_eq!(d.name, "find");
    assert_eq!(d.tab, "Replace");
}

#[test]
fn find_all_lists_matches_with_their_formulas() {
    let mut h = harness();
    h.state_mut().session.execute("cell.set", json!({"cell": "B1", "input": "=A1&\"alpha\""})).unwrap();
    press(&mut h, Key::L);
    // The dialog opens; set the term and press Find All.
    h.state_mut().dialog.as_mut().unwrap().values.insert("what".into(), json!("alpha"));
    h.get_by_label("Find All").click();
    h.run_steps(2);
    let r = h.state().dialog.as_ref().and_then(|d| d.result.clone()).expect("Find All ran");
    assert_eq!(r["count"], json!(3), "alpha, alphabet and the formula holding alpha: {r}");
    let list = r["results"].as_array().expect("a results list");
    assert!(list.iter().any(|m| m["cell"] == json!("A1")), "{r}");
    assert!(list.iter().any(|m| m["cell"] == json!("A3")), "{r}");
    assert!(list.iter().any(|m| m["cell"] == json!("B1") && m["formula"] == json!("=A1&\"alpha\"")), "the formula column: {r}");
}

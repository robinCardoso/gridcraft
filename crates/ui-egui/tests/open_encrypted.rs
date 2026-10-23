//! Opening a password-protected workbook asks for its password (#11).

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::{Session, core::CellRef, core::Value};
use gridcraft_ui_egui::{Inbox, Services, SheetApp};
use serde_json::json;

const PW: &str = "synthetic-pw-8842";

fn harness(services: Services) -> egui_kittest::Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, services),
    );
    h.run_steps(4);
    h
}

/// A workbook with 42 in A1, saved as `.xlsx` and encrypted with `PW`.
fn encrypted_bytes() -> Vec<u8> {
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    let xlsx = gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "book.xlsx").unwrap();
    gridcraft_xlsx::testing::encrypt_workbook(&xlsx, PW)
}

/// `encrypted_bytes()` written to `Secret.xlsx` in a fresh temp dir named after `tag`.
fn encrypted_file(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gridcraft-encrypted-ui-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Secret.xlsx");
    std::fs::write(&path, encrypted_bytes()).unwrap();
    path
}

fn dialog(h: &egui_kittest::Harness<'static, SheetApp>) -> Option<String> {
    h.state().dialog.as_ref().map(|d| d.name().to_string())
}

fn a1(h: &egui_kittest::Harness<'static, SheetApp>) -> Value {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::parse("A1").unwrap())
}

/// Steps the harness until the password dialog appears. A file that arrives through the inbox
/// (web picker, drag and drop) parses on a background thread now
/// (docs/plano-responsividade.md, Fase 2), so the dialog follows a few frames later than on the
/// synchronous desktop path.
fn wait_for_password_dialog(h: &mut egui_kittest::Harness<'static, SheetApp>) {
    for _ in 0..750 {
        if dialog(h).is_some() {
            // The dialog appeared during `logic`; let `ui` draw it before interacting.
            h.run_steps(2);
            return;
        }
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Steps the harness until a background open installs its workbook: the password dialog's OK
/// (and File › Open) parse on a thread now (Fase 2), so the workbook lands a few frames later.
fn wait_for_open(h: &mut egui_kittest::Harness<'static, SheetApp>) {
    for _ in 0..750 {
        if h.state().loading.is_none() && a1(h) != Value::Empty {
            h.run_steps(1);
            return;
        }
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn opening_an_encrypted_file_asks_for_its_password() {
    let path = encrypted_file("desktop");
    let p = path.to_string_lossy().to_string();
    let mut h = harness(Services::default());
    h.state_mut().open_path(&p);
    h.run_steps(3);
    assert_eq!(dialog(&h).as_deref(), Some("openPassword"));
    assert!(h.state().message.is_none(), "no error box: the dialog asks instead");
    assert!(h.query_by_role(Role::PasswordInput).is_some(), "the password is masked");

    // A wrong password says so, clears the field, and keeps the dialog and its file.
    h.get_by_role(Role::PasswordInput).click();
    h.run_steps(2);
    h.input_mut().events.push(egui::Event::Text("wrong-guess".into()));
    h.run_steps(2);
    h.get_by_label("   OK   ").click();
    // The click closes the dialog first; the background load then finds the password wrong and
    // reopens it (Fase 2: the parse runs on a thread).
    h.run_steps(2);
    wait_for_password_dialog(&mut h);
    let msg = h.state().message.as_ref().map(|(_, m)| m.clone()).unwrap_or_default();
    assert!(msg.contains(gridcraft_engine::io::WRONG_PASSWORD), "message: {:?}", h.state().message);
    assert!(!msg.contains("wrong-guess"), "the password is never quoted: {msg}");
    assert_eq!(dialog(&h).as_deref(), Some("openPassword"));
    assert_eq!(h.state().dialog.as_ref().and_then(|d| d.values.get("password")), Some(&json!("")));
    assert_eq!(h.state().dialog.as_ref().and_then(|d| d.values.get("path")), Some(&json!(p)));
    h.state_mut().message = None;

    // The right one opens the workbook: no save path, in Recent, with the notice.
    h.state_mut().dialog.as_mut().unwrap().values.insert("password".into(), json!(PW));
    h.get_by_label("   OK   ").click();
    wait_for_open(&mut h);
    assert_eq!(dialog(&h), None);
    assert_eq!(a1(&h), Value::Number(42.0));
    assert_eq!(h.state().session.active().unwrap().path, None);
    assert_eq!(h.state().ui.recent.first(), Some(&p));
    assert!(h.state().notice.as_ref().is_some_and(|n| n.text.contains("password-protected")));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn cancel_leaves_nothing_open() {
    let path = encrypted_file("cancel");
    let mut h = harness(Services::default());
    let before = h.state().session.documents().len();
    h.state_mut().open_path(&path.to_string_lossy());
    h.run_steps(3);
    assert_eq!(dialog(&h).as_deref(), Some("openPassword"));
    h.get_by_label(" Cancel ").click();
    h.run_steps(3);
    assert_eq!(dialog(&h), None);
    assert_eq!(h.state().session.documents().len(), before);
    assert!(h.state().message.is_none());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn a_file_picked_on_the_web_asks_for_its_password_too() {
    let inbox: Inbox = Default::default();
    let mut h = harness(Services { inbox: Some(inbox.clone()), ..Default::default() });
    inbox.lock().unwrap().push(("Secret.xlsx".into(), encrypted_bytes()));
    wait_for_password_dialog(&mut h);
    assert_eq!(dialog(&h).as_deref(), Some("openPassword"));
    assert!(h.state().dialog.as_ref().is_some_and(|d| d.values.contains_key("base64")));
    h.state_mut().dialog.as_mut().unwrap().values.insert("password".into(), json!(PW));
    h.get_by_label("   OK   ").click();
    wait_for_open(&mut h);
    assert_eq!(dialog(&h), None, "the dialog stays open with: {:?}", h.state().message);
    assert_eq!(a1(&h), Value::Number(42.0));
    assert_eq!(h.state().session.active().unwrap().path, None);
}

#[test]
fn an_untouched_password_field_is_incorrect_not_required() {
    let path = encrypted_file("untouched");
    let mut h = harness(Services::default());
    h.state_mut().open_path(&path.to_string_lossy());
    h.run_steps(3);
    h.get_by_label("   OK   ").click();
    // The click closes the dialog first; the background load reopens it (empty password = wrong).
    h.run_steps(2);
    wait_for_password_dialog(&mut h);
    let msg = h.state().message.as_ref().map(|(_, m)| m.clone()).unwrap_or_default();
    assert!(msg.contains(gridcraft_engine::io::WRONG_PASSWORD), "message: {:?}", h.state().message);
    assert_eq!(dialog(&h).as_deref(), Some("openPassword"));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn the_backstage_closes_once_the_password_opens_the_workbook() {
    let path = encrypted_file("backstage");
    let mut h = harness(Services::default());
    h.state_mut().backstage = Some(gridcraft_ui_egui::backstage::Backstage::new(gridcraft_ui_egui::backstage::Page::Open));
    h.state_mut().open_path(&path.to_string_lossy());
    h.run_steps(3);
    h.state_mut().dialog.as_mut().unwrap().values.insert("password".into(), json!(PW));
    h.get_by_label("   OK   ").click();
    wait_for_open(&mut h);
    assert_eq!(a1(&h), Value::Number(42.0));
    assert!(h.state().backstage.is_none(), "the opened workbook shows, not the Backstage");
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn app_open_over_the_control_channel_never_opens_a_dialog() {
    use gridcraft_ui_egui::control::{Outcome, handle};
    let path = encrypted_file("control");
    let p = path.to_string_lossy().to_string();
    let mut h = harness(Services::default());
    let ctx = h.ctx.clone();
    let reply = |o: Outcome| match o {
        Outcome::Done(v) => v,
        Outcome::Screenshot { .. } => json!(null),
    };
    let r = reply(handle(h.state_mut(), &ctx, "app.open", &json!({"path": p})));
    assert_eq!(r["ok"], json!(false), "{r}");
    assert!(r["error"].as_str().unwrap_or_default().contains(gridcraft_engine::io::PASSWORD_REQUIRED), "{r}");
    assert_eq!(dialog(&h), None, "programmatic calls never open dialogs");
    let r = reply(handle(h.state_mut(), &ctx, "app.open", &json!({"path": p, "password": PW})));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["result"]["title"], json!("Secret.xlsx"));
    assert_eq!(a1(&h), Value::Number(42.0));
    assert_eq!(h.state().ui.recent.first(), Some(&p));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

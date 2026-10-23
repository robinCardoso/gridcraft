//! UI-path regression: paste codes into Tabela3 col A must fill Descrição (C) after the calc job.
#![allow(clippy::disallowed_methods)]

use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::{Services, SheetApp, jobs};
use serde_json::json;

fn xlsm_path() -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        r"c:\Users\Robson\Downloads\10 LISTA SC para Outros.xlsm",
        r"c:\Users\Robson\source\grid-excel\10 LISTA SC para Outros.xlsm",
    ];
    CANDIDATES.iter().map(std::path::Path::new).find(|p| p.exists()).map(std::path::Path::to_path_buf)
}

fn wait_for_job(app: &mut SheetApp, ctx: &egui::Context) -> usize {
    for _ in 0..5000 {
        let done = app.jobs.poll(ctx);
        if !done.is_empty() {
            let n = done.len();
            for r in done {
                app.apply_job(r);
            }
            return n;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    0
}

fn value_at(app: &SheetApp, a1: &str) -> Value {
    let d = app.session.active().expect("doc");
    let c = CellRef::parse(a1).unwrap();
    d.wb.active().unwrap().value(c)
}

fn drain_calc(app: &mut SheetApp, ctx: &egui::Context) {
    for _ in 0..10_000 {
        app.after_engine();
        wait_for_job(app, ctx);
        let pending = app.session.active().is_some_and(|d| d.recalc_pending);
        let uid = app.session.active().map(|d| d.uid).unwrap_or(0);
        if !pending && !app.jobs.is_calculating(uid) {
            break;
        }
    }
}

#[test]
fn ui_paste_a6_a13_fills_c_after_job() {
    let Some(path) = xlsm_path() else {
        eprintln!("skip: xlsm not found");
        return;
    };
    let mut session = Session::new();
    session.execute("file.open", json!({"path": path})).unwrap();
    let pedido = session
        .doc()
        .unwrap()
        .wb
        .sheets
        .iter()
        .position(|sh| sh.name.eq_ignore_ascii_case("Pedido"))
        .expect("Pedido");
    session.execute("sheet.activate", json!({"index": pedido})).unwrap();

    let mut app = SheetApp::new(session, Services::default());
    // Real UI thresholds for this ~59k-formula book: defer_structural flips on.
    app.job_thresholds = jobs::Thresholds {
        save_cells: 50_000,
        calc_formulas: 5_000,
    };
    app.after_engine();
    assert!(
        app.session.active().is_some_and(|d| d.defer_structural),
        "big book should defer structural/cascade"
    );

    let codes = "11002048\n11002148\n11002248\n11002948\n11003048\n11003148\n11003248\n11003948\n";
    app.run("edit.paste", json!({"at": "A6", "text": codes})).expect("paste");
    app.after_engine();

    let ctx = egui::Context::default();
    drain_calc(&mut app, &ctx);

    let c6 = value_at(&app, "C6");
    let c13 = value_at(&app, "C13");
    assert!(
        matches!(&c6, Value::Text(t) if !t.is_empty()),
        "C6 after UI paste+job: {c6:?}"
    );
    assert!(
        matches!(&c13, Value::Text(t) if !t.is_empty()),
        "C13 after UI paste+job: {c13:?}"
    );
}

#[test]
fn ui_calc_job_bumps_revision_so_text_cache_drops_stale_empty() {
    let Some(path) = xlsm_path() else {
        return;
    };
    let mut session = Session::new();
    session.execute("file.open", json!({"path": path})).unwrap();
    let pedido = session
        .doc()
        .unwrap()
        .wb
        .sheets
        .iter()
        .position(|sh| sh.name.eq_ignore_ascii_case("Pedido"))
        .expect("Pedido");
    session.execute("sheet.activate", json!({"index": pedido})).unwrap();

    let mut app = SheetApp::new(session, Services::default());
    app.job_thresholds = jobs::Thresholds {
        save_cells: 50_000,
        calc_formulas: 5_000,
    };
    app.after_engine();

    let codes = "11002048\n11002148\n11002248\n11002948\n11003048\n11003148\n11003248\n11003948\n\
11011030\n11012030\n11013040\n11013140\n11013240\n11013940\n11014040\n11014140\n11014240\n11014940\n";
    app.run("edit.paste", json!({"at": "A6", "text": codes})).expect("paste");
    app.after_engine();
    let rev_after_paste = app.session.active().map(|d| d.revision).unwrap_or(0);
    assert!(
        app.session.active().is_some_and(|d| d.recalc_pending) || app.jobs.is_calculating(app.session.active().unwrap().uid),
        "expand paste owes a calc job"
    );

    let ctx = egui::Context::default();
    drain_calc(&mut app, &ctx);

    let rev_after_job = app.session.active().map(|d| d.revision).unwrap_or(0);
    assert!(
        rev_after_job > rev_after_paste,
        "calc job must bump revision (was {rev_after_paste}, now {rev_after_job}) so text_cache invalidates"
    );
    assert!(
        matches!(value_at(&app, "C6"), Value::Text(ref t) if !t.is_empty()),
        "C6 after job"
    );
    assert!(
        matches!(value_at(&app, "C14"), Value::Text(ref t) if !t.is_empty()),
        "C14 after job"
    );
}

#[test]
fn ui_paste_with_excel_html_xnum_fills_c() {
    let Some(path) = xlsm_path() else {
        return;
    };
    let mut session = Session::new();
    session.execute("file.open", json!({"path": path})).unwrap();
    let pedido = session
        .doc()
        .unwrap()
        .wb
        .sheets
        .iter()
        .position(|sh| sh.name.eq_ignore_ascii_case("Pedido"))
        .expect("Pedido");
    session.execute("sheet.activate", json!({"index": pedido})).unwrap();

    let mut app = SheetApp::new(session, Services::default());
    app.job_thresholds = jobs::Thresholds {
        save_cells: 50_000,
        calc_formulas: 5_000,
    };
    app.after_engine();

    // Excel-style clipboard: plain text + HTML x:num (UI paste_external path).
    let text = "11002048\n11002148\n11002248\n11002948\n11003048\n11003148\n11003248\n11003948\n";
    let html = r#"<table>
<tr><td x:num="11002048">11002048</td></tr>
<tr><td x:num="11002148">11002148</td></tr>
<tr><td x:num="11002248">11002248</td></tr>
<tr><td x:num="11002948">11002948</td></tr>
<tr><td x:num="11003048">11003048</td></tr>
<tr><td x:num="11003148">11003148</td></tr>
<tr><td x:num="11003248">11003248</td></tr>
<tr><td x:num="11003948">11003948</td></tr>
</table>"#;
    app.run("edit.paste", json!({"at": "A6", "text": text, "html": html})).expect("paste");
    app.after_engine();

    let a6 = value_at(&app, "A6");
    eprintln!("A6 after HTML paste: {a6:?}");

    let ctx = egui::Context::default();
    drain_calc(&mut app, &ctx);

    let c6 = value_at(&app, "C6");
    assert!(
        matches!(&c6, Value::Text(t) if !t.is_empty()),
        "C6 after HTML x:num paste: A6={a6:?} C6={c6:?}"
    );
}

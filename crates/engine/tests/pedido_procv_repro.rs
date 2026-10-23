//! Regression: aba Pedido em "10 LISTA SC para Outros.xlsm" (Tabela3 + PROCV).
//! Sintoma: códigos colados em A (linhas do corpo) não atualizam C–L; expand abaixo sim.
#![allow(clippy::disallowed_methods)]

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::Session;
use serde_json::json;

fn xlsm_path() -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        r"c:\Users\Robson\Downloads\10 LISTA SC para Outros.xlsm",
        r"c:\Users\Robson\source\grid-excel\10 LISTA SC para Outros.xlsm",
    ];
    CANDIDATES
        .iter()
        .map(std::path::Path::new)
        .find(|p| p.exists())
        .map(std::path::Path::to_path_buf)
}

fn v(s: &Session, a1: &str) -> Value {
    let d = s.doc().unwrap();
    let c = CellRef::parse(a1).unwrap();
    d.wb.active().unwrap().value(c)
}

fn sheet_index(s: &Session, name: &str) -> usize {
    let d = s.doc().unwrap();
    d.wb
        .sheets
        .iter()
        .position(|sh| sh.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("sheet {name:?} not found"))
}

#[test]
fn pedido_procv_updates_after_paste_in_table_body() {
    let Some(path) = xlsm_path() else {
        eprintln!("skip: xlsm not found");
        return;
    };
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    let pedido = sheet_index(&s, "Pedido");
    s.execute("sheet.activate", json!({"index": pedido})).unwrap();

    let codes = "11002048\n11002148\n11002248\n11002348\n11002448\n11002548\n11002648\n11002748\n11002848\n11011030\n";
    s.execute("edit.paste", json!({"at": "A6", "text": codes})).unwrap();

    assert!(
        matches!(v(&s, "C6"), Value::Text(ref t) if !t.is_empty()),
        "C6 must show Descrição after code in A6"
    );
    assert!(
        matches!(v(&s, "C15"), Value::Text(ref t) if !t.is_empty()),
        "expanded row C15 must calculate after paste past table end"
    );
}

#[test]
fn pedido_deferred_table_expand_job_uses_paste_seeds() {
    let Some(path) = xlsm_path() else {
        return;
    };
    let codes = (0..12)
        .map(|i| format!("110{:05}", 2048 + i * 100))
        .collect::<Vec<_>>()
        .join("\n");

    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    let pedido = sheet_index(&s, "Pedido");
    s.execute("sheet.activate", json!({"index": pedido})).unwrap();
    s.doc_mut().unwrap().defer_structural = true;

    s.execute("edit.paste", json!({"at": "A6", "text": codes})).unwrap();
    let d = s.doc().unwrap();
    assert!(d.recalc_pending);
    let cascade = d.recalc_cascade.clone().expect("structural defer records paste seeds");
    assert!(!cascade.is_empty(), "seeds must include pasted input cells");
    let c6 = (pedido, CellRef::parse("C6").unwrap());
    assert!(cascade.contains(&c6), "body formula C6 must be a cascade seed");

    let (wb, _, _) = gridcraft_engine::recalc_document(&d.wb, d.calc.clone(), None, None, d.calc.seeds(), Some(cascade));
    let sh = wb.sheet(pedido).unwrap();
    let c6 = sh.value(CellRef::parse("C6").unwrap());
    let c15 = sh.value(CellRef::parse("C15").unwrap());
    assert!(matches!(c6, Value::Text(ref t) if !t.is_empty()));
    assert!(matches!(c15, Value::Text(ref t) if !t.is_empty()));
}

/// UI path: big workbook + `defer_structural`, paste A6:A23 (inside Tabela3 + expand).
#[test]
fn pedido_deferred_paste_a6_a23_fills_table_body_and_expanded_rows() {
    let Some(path) = xlsm_path() else {
        return;
    };
    let codes = "11002048\n11002148\n11002248\n11002948\n11003048\n11003148\n11003248\n11003948\n\
11011030\n11012030\n11013040\n11013140\n11013240\n11013940\n11014040\n11014140\n11014240\n11014940\n";

    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    let pedido = sheet_index(&s, "Pedido");
    s.execute("sheet.activate", json!({"index": pedido})).unwrap();
    s.doc_mut().unwrap().defer_structural = true;

    s.execute("edit.paste", json!({"at": "A6", "text": codes})).unwrap();
    let d = s.doc().unwrap();
    assert!(d.recalc_pending, "structural table expand defers recalc");
    let cascade = d.recalc_cascade.clone().expect("structural defer records paste seeds");
    assert!(!cascade.is_empty());
    for row in 6..=13u32 {
        let c = (pedido, CellRef::new(row - 1, 2)); // C6..C13
        assert!(cascade.contains(&c), "cascade must seed C{row}");
    }

    assert!(
        matches!(v(&s, "C6"), Value::Text(ref t) if t.is_empty()),
        "C6 stale until job: {:?}",
        v(&s, "C6")
    );

    let seeds = d.calc.seeds();
    let (wb, calc, cells) =
        gridcraft_engine::recalc_document(&d.wb, d.calc.clone(), None, None, seeds, Some(cascade));
    assert!(cells > 0, "job must evaluate formulas");
    let sh = wb.sheet(pedido).unwrap();
    let c6 = sh.value(CellRef::parse("C6").unwrap());
    let c13 = sh.value(CellRef::parse("C13").unwrap());
    let c14 = sh.value(CellRef::parse("C14").unwrap());
    let c23 = sh.value(CellRef::parse("C23").unwrap());
    assert!(matches!(c6, Value::Text(ref t) if !t.is_empty()), "C6 after job: {c6:?}");
    assert!(matches!(c13, Value::Text(ref t) if !t.is_empty()), "C13 after job: {c13:?}");
    assert!(matches!(c14, Value::Text(ref t) if !t.is_empty()), "C14 after job: {c14:?}");
    assert!(matches!(c23, Value::Text(ref t) if !t.is_empty()), "C23 after job: {c23:?}");
    assert!(calc.graph_is_current());
}

/// Paste only inside Tabela3 body (no row expand).
#[test]
fn pedido_deferred_paste_inside_table_body_without_expand() {
    let Some(path) = xlsm_path() else {
        return;
    };
    let codes = "11002048\n11002148\n11002248\n11002948\n11003048\n11003148\n11003248\n11003948\n";

    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    let pedido = sheet_index(&s, "Pedido");
    s.execute("sheet.activate", json!({"index": pedido})).unwrap();
    s.doc_mut().unwrap().defer_structural = true;

    s.execute("edit.paste", json!({"at": "A6", "text": codes})).unwrap();
    let d = s.doc().unwrap();
    assert!(
        !d.recalc_pending || d.recalc_cascade.as_ref().is_some_and(|c| !c.is_empty()),
        "paste must defer with cascade or recalc inline"
    );
    if d.recalc_pending {
        let cascade = d.recalc_cascade.clone().unwrap();
        let (wb, _, _) =
            gridcraft_engine::recalc_document(&d.wb, d.calc.clone(), None, None, d.calc.seeds(), Some(cascade));
        let sh = wb.sheet(pedido).unwrap();
        let c6 = sh.value(CellRef::parse("C6").unwrap());
        assert!(matches!(c6, Value::Text(ref t) if !t.is_empty()), "C6: {c6:?}");
    } else {
        assert!(matches!(v(&s, "C6"), Value::Text(ref t) if !t.is_empty()));
    }
}

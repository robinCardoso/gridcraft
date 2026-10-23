//! Regression: aba Pedido em "10 LISTA SC para Outros.xlsm" (Tabela3 A5:L14 + PROCV).
//! Sintoma reportado: códigos colados em A6–A14 não atualizam B–L; linhas 15+ sim.
#![allow(clippy::disallowed_methods)]

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::Session;
use serde_json::json;

const PATH: &str = r"c:\Users\Robson\Downloads\10 LISTA SC para Outros.xlsm";

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
    if !std::path::Path::new(PATH).exists() {
        eprintln!("skip: {PATH} not found");
        return;
    }
    let mut s = Session::new();
    s.execute("file.open", json!({"path": PATH})).unwrap();
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
        "expanded row C15 must calculate after paste past row 14"
    );
}

#[test]
fn pedido_deferred_table_expand_job_uses_paste_seeds() {
    if !std::path::Path::new(PATH).exists() {
        return;
    }
    let codes = (0..12)
        .map(|i| format!("110{:05}", 2048 + i * 100))
        .collect::<Vec<_>>()
        .join("\n");

    let mut s = Session::new();
    s.execute("file.open", json!({"path": PATH})).unwrap();
    let pedido = sheet_index(&s, "Pedido");
    s.execute("sheet.activate", json!({"index": pedido})).unwrap();
    s.doc_mut().unwrap().defer_structural = true;

    s.execute("edit.paste", json!({"at": "A6", "text": codes})).unwrap();
    let d = s.doc().unwrap();
    assert!(d.recalc_pending);
    let cascade = d.recalc_cascade.clone().expect("structural defer records paste seeds");
    assert!(!cascade.is_empty(), "seeds must include pasted input cells");

    let (wb, _, _) = gridcraft_engine::recalc_document(&d.wb, d.calc.clone(), None, None, d.calc.seeds(), Some(cascade));
    let sh = wb.sheet(pedido).unwrap();
    let c6 = sh.value(CellRef::parse("C6").unwrap());
    let c15 = sh.value(CellRef::parse("C15").unwrap());
    assert!(matches!(c6, Value::Text(ref t) if !t.is_empty()));
    assert!(matches!(c15, Value::Text(ref t) if !t.is_empty()));
}

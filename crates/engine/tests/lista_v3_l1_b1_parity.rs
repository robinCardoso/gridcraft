//! COTACAO!M1 caches E1-only ref match; L1 must not re-scan refEq when only B1/B2 change (n=1).
#![allow(clippy::disallowed_methods)]

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::Session;
use serde_json::json;

fn xlsx_path() -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        r"c:\Users\Robson\source\grid-excel\LISTA TESTE V3.xlsx",
        r"c:\Users\Robson\source\grid-excel\gridcraft\..\LISTA TESTE V3.xlsx",
    ];
    CANDIDATES
        .iter()
        .map(std::path::Path::new)
        .find(|p| p.is_file())
        .map(std::path::Path::to_path_buf)
}

fn num(s: &Session, a1: &str) -> f64 {
    match cell(s, a1) {
        Value::Number(n) => n,
        other => panic!("{a1}: expected number, got {other:?}"),
    }
}

fn cell(s: &Session, a1: &str) -> Value {
    let d = s.doc().unwrap();
    let c = CellRef::parse(a1).unwrap();
    d.wb.active().unwrap().value(c)
}

fn text(s: &Session, a1: &str) -> String {
    match cell(s, a1) {
        Value::Text(t) => t.to_string(),
        Value::Empty => String::new(),
        Value::Number(n) => {
            if (n - n.round()).abs() < 1e-9 {
                format!("{}", n as i64)
            } else {
                n.to_string()
            }
        }
        other => panic!("{a1}: unexpected {other:?}"),
    }
}

fn setup_cotacao(s: &mut Session) {
    let cot = s
        .doc()
        .unwrap()
        .wb
        .sheets
        .iter()
        .position(|sh| sh.name.eq_ignore_ascii_case("COTACAO"))
        .unwrap();
    s.execute("sheet.activate", json!({"index": cot})).unwrap();
}

#[test]
fn lista_v3_l1_m1_stable_on_b1_change() {
    let Some(path) = xlsx_path() else {
        eprintln!("skip: LISTA TESTE V3.xlsx not found");
        return;
    };

    let backup = path.with_file_name("LISTA TESTE V3.pre-l1-split.xlsx");
    if backup.is_file() {
        let mut ref_s = Session::new();
        ref_s
            .execute("file.open", json!({"path": backup.to_string_lossy()}))
            .unwrap();
        setup_cotacao(&mut ref_s);
        ref_s.execute("cell.set", json!({"cell": "E1", "input": "87428610"})).unwrap();
        ref_s.execute("cell.set", json!({"cell": "B1", "input": "SC"})).unwrap();
        ref_s.execute("cell.set", json!({"cell": "B2", "input": "SC"})).unwrap();
        ref_s.execute("formulas.calculateNow", json!({})).unwrap();
        eprintln!("pre-split L1={:?}", text(&ref_s, "L1"));
    }

    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    setup_cotacao(&mut s);
    s.execute("cell.set", json!({"cell": "E1", "input": "87428610"})).unwrap();
    s.execute("cell.set", json!({"cell": "B1", "input": "SC"})).unwrap();
    s.execute("cell.set", json!({"cell": "B2", "input": "SC"})).unwrap();
    s.execute("formulas.calculateNow", json!({})).unwrap();

    let m1 = text(&s, "M1");
    let l1 = text(&s, "L1");
    eprintln!("after F9: M1={m1:?} L1={l1:?}");
    let m1_num = num(&s, "M1");
    assert!((m1_num - 5101.0).abs() < 1e-9, "M1 should cache row index 5101, got {m1_num}");
    assert_eq!(l1, "5101", "L1 filial SC (M1={m1})");

    s.execute("cell.set", json!({"cell": "B1", "input": "PR"})).unwrap();
    assert!((num(&s, "M1") - m1_num).abs() < 1e-9, "M1 must not change when only B1 changes (E1 fixed)");

    // Paridade com backup pré-split: mesmo E1/B2 e B1=PR.
    let backup = path.with_file_name("LISTA TESTE V3.pre-l1-split.xlsx");
    if backup.is_file() {
        let mut ref_s = Session::new();
        ref_s.execute("file.open", json!({"path": backup.to_string_lossy()})).unwrap();
        setup_cotacao(&mut ref_s);
        ref_s.execute("cell.set", json!({"cell": "E1", "input": "87428610"})).unwrap();
        ref_s.execute("cell.set", json!({"cell": "B1", "input": "PR"})).unwrap();
        ref_s.execute("cell.set", json!({"cell": "B2", "input": "SC"})).unwrap();
        ref_s.execute("formulas.calculateNow", json!({})).unwrap();
        assert_eq!(text(&s, "L1"), text(&ref_s, "L1"), "L1(B1=PR) vs pre-split formula");
    }
}

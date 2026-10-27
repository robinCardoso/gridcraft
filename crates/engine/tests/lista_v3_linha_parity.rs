//! Paridade ORCAMENTO[Linha] otimizado (`refEq` + fast path `n=1`) vs baseline da planilha.
//! Baseline capturado com `LISTA TESTE V3.pre-linha.xlsx` (fórmula `achou` + XMATCH sempre).
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

fn cell(s: &Session, a1: &str) -> Value {
    let d = s.doc().unwrap();
    let c = CellRef::parse(a1).unwrap();
    d.wb.active().unwrap().value(c)
}

fn num(s: &Session, a1: &str) -> f64 {
    match cell(s, a1) {
        Value::Number(n) => n,
        other => panic!("{a1}: expected number, got {other:?}"),
    }
}

fn text(s: &Session, a1: &str) -> String {
    match cell(s, a1) {
        Value::Text(t) => t.to_string(),
        Value::Empty => String::new(),
        other => panic!("{a1}: expected text, got {other:?}"),
    }
}

/// Valores GridCraft com E1=87428610, B1=B2=SC e D16=MB006 (baseline `pre-linha` / fórmula `achou`).
const BASELINE_L16_MB006: f64 = 7.0;
const BASELINE_E16_MB006: &str = "CABEÇOTE - OM 352 A - 30° (1318)";
const BASELINE_J16_MB006: f64 = 3503.982_558_139_535_3;
/// D16=50017390 (referência original da linha 16), mesmos E1/B1/B2.
const BASELINE_L16_REF_ROW: f64 = 6110.0;

#[test]
fn lista_v3_linha_ref_eq_parity_mb006() {
    let Some(path) = xlsx_path() else {
        eprintln!("skip lista_v3_linha_refEq_parity_mb006: LISTA TESTE V3.xlsx not found");
        return;
    };

    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    s.execute("sheet.activate", json!({"name": "COTACAO"})).unwrap();
    s.execute("formulas.calculateNow", json!({})).unwrap();

    s.execute("cell.set", json!({"cell": "E1", "input": "87428610"})).unwrap();
    s.execute("cell.set", json!({"cell": "B1", "input": "SC"})).unwrap();
    s.execute("cell.set", json!({"cell": "B2", "input": "SC"})).unwrap();
    s.execute("formulas.calculateNow", json!({})).unwrap();

    let l1 = s.execute("cell.get", json!({"cell": "L1"})).unwrap();
    eprintln!("L1 after E1/B1/B2: {:?}", l1.get("value"));

    s.execute("cell.set", json!({"cell": "D16", "input": "MB006"})).unwrap();

    let l16 = num(&s, "L16");
    let e16 = text(&s, "E16");
    let j16 = num(&s, "J16");
    eprintln!("D16=MB006 → L16={l16} E16={e16:?} J16={j16}");

    assert_eq!(l16, BASELINE_L16_MB006, "Linha index drift");
    assert_eq!(e16, BASELINE_E16_MB006, "Descricao drift");
    assert_eq!(j16, BASELINE_J16_MB006, "Preco_Unit drift");

    let f = s.execute("cell.get", json!({"cell": "L16"})).unwrap();
    let formula = f.get("formula").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        !formula.contains("achou"),
        "Linha should use refEq peelable equality, not achou; len={}",
        formula.len()
    );

    s.execute("cell.set", json!({"cell": "D16", "input": "50017390"})).unwrap();
    assert_eq!(num(&s, "L16"), BASELINE_L16_REF_ROW, "original D16 ref row");
}

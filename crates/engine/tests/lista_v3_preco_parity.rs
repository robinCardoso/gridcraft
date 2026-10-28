//! Paridade `ORCAMENTO[Preco_Unit]` (J) e `Total` (K) após `ORCAMENTO[Linha]` (L) com `refEq`.
//! Baseline: `LISTA TESTE V3.pre-preco.xlsx` / GridCraft com E1=87428610, B1=B2=SC.
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

fn text_cell(s: &Session, a1: &str) -> String {
    match cell(s, a1) {
        Value::Text(t) => t.to_string(),
        Value::Empty => String::new(),
        other => panic!("{a1}: expected text, got {other:?}"),
    }
}

fn setup_e1_b1_b2(s: &mut Session) {
    s.execute("cell.set", json!({"cell": "E1", "input": "87428610"})).unwrap();
    s.execute("cell.set", json!({"cell": "B1", "input": "SC"})).unwrap();
    s.execute("cell.set", json!({"cell": "B2", "input": "SC"})).unwrap();
    s.execute("formulas.calculateNow", json!({})).unwrap();
}

/// D16=MB006 (mesmo baseline que `lista_v3_linha_parity.rs`).
const BASELINE_L16_MB006: f64 = 7.0;
const BASELINE_J16_MB006: f64 = 3503.982_558_139_535_3;
const BASELINE_K16_MB006: f64 = 0.0; // Qtd vazia no corpus → Total=0

/// D16=87428610 com E1/B1/B2 acima (referência do cabeçalho E1).
const BASELINE_L16_E1_REF: f64 = 5101.0;
const BASELINE_J16_E1_REF: f64 = 153.532_391_997_599_97;
const BASELINE_K16_E1_REF: f64 = 0.0;
const BASELINE_E16_E1_REF: &str = "BRONZINA DE BIELA (025) MB OM-352 OM-352";

#[test]
fn lista_v3_preco_parity_mb006_and_e1_ref() {
    let Some(path) = xlsx_path() else {
        eprintln!("skip lista_v3_preco_parity: LISTA TESTE V3.xlsx not found");
        return;
    };

    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    s.execute("sheet.activate", json!({"name": "COTACAO"})).unwrap();
    s.execute("formulas.calculateNow", json!({})).unwrap();

    setup_e1_b1_b2(&mut s);

    s.execute("cell.set", json!({"cell": "D16", "input": "MB006"})).unwrap();
    s.execute("formulas.calculateNow", json!({})).unwrap();

    let l16 = num(&s, "L16");
    let j16 = num(&s, "J16");
    let k16 = num(&s, "K16");
    eprintln!("D16=MB006 → L16={l16} J16={j16} K16={k16}");

    assert_eq!(l16, BASELINE_L16_MB006, "Linha");
    assert_eq!(j16, BASELINE_J16_MB006, "Preco_Unit");
    assert_eq!(k16, BASELINE_K16_MB006, "Total");

    let jf = s.execute("cell.get", json!({"cell": "J16"})).unwrap();
    let j_formula = jf.get("formula").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        !j_formula.contains("achou") && !j_formula.contains("refEq"),
        "Preco_Unit should INDEX via Linha, not relookup; len={}",
        j_formula.len()
    );
    assert!(
        j_formula.contains("Preco_Venda") && j_formula.contains("Preco_Revenda"),
        "Preco_Unit filial/cliente branch"
    );

    s.execute("cell.set", json!({"cell": "D16", "input": "87428610"})).unwrap();
    s.execute("formulas.calculateNow", json!({})).unwrap();

    let l16b = num(&s, "L16");
    let j16b = num(&s, "J16");
    let k16b = num(&s, "K16");
    let e16b = text_cell(&s, "E16");
    eprintln!(
        "D16=87428610 → L16={l16b} J16={j16b} K16={k16b} E16={e16b:?}"
    );

    assert_eq!(l16b, BASELINE_L16_E1_REF, "Linha for E1 ref");
    assert_eq!(j16b, BASELINE_J16_E1_REF, "Preco_Unit for E1 ref");
    assert_eq!(k16b, BASELINE_K16_E1_REF, "Total for E1 ref");
    assert_eq!(e16b, BASELINE_E16_E1_REF, "Descricao for E1 ref");
}

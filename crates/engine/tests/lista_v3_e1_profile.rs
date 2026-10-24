//! Profile: edit COTACAO!E1 on LISTA TESTE V3 (user report ~20391 formulas, ~2 min).
//! Run (release recommended):
//!   $env:GRIDCRAFT_CALC_STATS='1'
//!   cargo test -p gridcraft-engine lista_v3_e1_edit_profile --release -- --nocapture
#![allow(clippy::disallowed_methods)]

use std::time::Instant;

use gridcraft_engine::Session;
use serde_json::json;

fn xlsx_path() -> Option<String> {
    const CANDIDATES: &[&str] = &[
        r"c:\Users\Robson\source\grid-excel\LISTA TESTE V3.xlsx",
        r"c:\Users\Robson\source\grid-excel\gridcraft\..\LISTA TESTE V3.xlsx",
    ];
    CANDIDATES
        .iter()
        .map(std::path::Path::new)
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

fn sheet_index(s: &Session, name: &str) -> usize {
    s.doc()
        .unwrap()
        .wb
        .sheets
        .iter()
        .position(|sh| sh.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("sheet {name:?} not found"))
}

#[test]
fn lista_v3_e1_edit_profile() {
    let Some(path) = xlsx_path() else {
        eprintln!("skip lista_v3_e1_edit_profile: LISTA TESTE V3.xlsx not found");
        return;
    };

    if std::env::var_os("GRIDCRAFT_CALC_STATS").is_none() {
        eprintln!("hint: set GRIDCRAFT_CALC_STATS=1 for histogram output");
    }

    let t_open = Instant::now();
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    eprintln!("open+recalc_on_load: {:.2?}", t_open.elapsed());

    let cotacao = sheet_index(&s, "COTACAO");
    s.execute("sheet.activate", json!({"index": cotacao})).unwrap();

    let (graph_total, sheet_formulas, recalc_pending) = {
        let d = s.doc().unwrap();
        (
            d.calc.graph.len(),
            d.wb.sheet(cotacao).map(|sh| sh.cells.iter().filter(|(_, c)| c.formula.is_some()).count()).unwrap_or(0),
            d.recalc_pending,
        )
    };

    let e1 = s.execute("cell.get", json!({"cell": "E1"})).unwrap();
    eprintln!(
        "COTACAO: graph_nodes={graph_total} sheet_formula_cells={sheet_formulas} E1 type={:?} value={:?} formula={}",
        e1.get("type"),
        e1.get("value"),
        e1.get("formula").and_then(|v| v.as_str()).unwrap_or("-")
    );

    // Sample one heavy body formula (ORCAMENTO row ~16 → Excel row 16 = index 15).
    for a1 in ["L16", "E16", "J16"] {
        let g = s.execute("cell.get", json!({"cell": a1})).unwrap();
        let f = g.get("formula").and_then(|v| v.as_str()).unwrap_or("");
        eprintln!("  {a1} value={:?} formula_len={}", g.get("value"), f.len());
    }

    // Optional: full F9 before E1 (slow on ~640k formulas). Set LISTA_V3_FULL_WARM=1 to mimic a book already calculated.
    if std::env::var_os("LISTA_V3_FULL_WARM").is_some() && recalc_pending {
        eprintln!("LISTA_V3_FULL_WARM: calculateNow before E1...");
        let t = Instant::now();
        s.execute("formulas.calculateNow", json!({})).unwrap();
        eprintln!("  calculateNow: {:.2?}, cells={}", t.elapsed(), s.doc().unwrap().calc.last_recalc_cells);
    } else {
        eprintln!("skip full warm (set LISTA_V3_FULL_WARM=1 for post-F9 baseline); recalc_pending={recalc_pending}");
    }

    let before_e1 = s.execute("cell.get", json!({"cell": "E1"})).unwrap();
    let new_input = match before_e1.get("value") {
        Some(v) if v.is_number() => "78126601",
        Some(v) if v.as_str().is_some_and(|t| !t.is_empty()) => "78126601",
        _ => "78126600",
    };

    eprintln!("--- cell.set E1 = {new_input} (GRIDCRAFT_CALC_STATS on) ---");
    let t_edit = Instant::now();
    s.execute("cell.set", json!({"cell": "E1", "input": new_input})).unwrap();
    let edit_elapsed = t_edit.elapsed();
    let after = s.doc().unwrap();
    eprintln!(
        "edit E1 wall: {:.2?} | last_recalc_cells={} | recalc_pending={} | graph_len={}",
        edit_elapsed,
        after.calc.last_recalc_cells,
        after.recalc_pending,
        after.calc.graph.len()
    );

    // Compare: single-row Referencia edit (should be much smaller dirty set if graph is tight).
    eprintln!("--- cell.set D16 (one ORCAMENTO Referencia) for cascade size compare ---");
    let t_one = Instant::now();
    s.execute("cell.set", json!({"cell": "D16", "input": "MB006"})).unwrap();
    eprintln!(
        "edit D16 wall: {:.2?} | last_recalc_cells={}",
        t_one.elapsed(),
        s.doc().unwrap().calc.last_recalc_cells
    );

    let l16 = s.execute("cell.get", json!({"cell": "L16"})).unwrap();
    eprintln!("after D16=MB006: L16 value={:?}", l16.get("value"));
}

/// Shift+F9 on COTACAO ≈ barra "Calculando… N / 20391" após abrir o livro.
#[test]
fn lista_v3_cotacao_calculate_sheet_profile() {
    let Some(path) = xlsx_path() else {
        return;
    };
    if std::env::var_os("GRIDCRAFT_CALC_STATS").is_none() {
        eprintln!("hint: set GRIDCRAFT_CALC_STATS=1");
    }

    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    s.execute("sheet.activate", json!({"index": sheet_index(&s, "COTACAO")})).unwrap();

    eprintln!("--- formulas.calculateSheet (COTACAO uncomputed) ---");
    let t = Instant::now();
    s.execute("formulas.calculateSheet", json!({})).unwrap();
    let d = s.doc().unwrap();
    eprintln!(
        "calculateSheet wall: {:.2?} | last_recalc_cells={}",
        t.elapsed(),
        d.calc.last_recalc_cells
    );

    eprintln!("--- E1 edit after sheet fully calculated ---");
    let t = Instant::now();
    s.execute("cell.set", json!({"cell": "E1", "input": "78126602"})).unwrap();
    eprintln!(
        "E1 after warm sheet: {:.2?} | last_recalc_cells={}",
        t.elapsed(),
        s.doc().unwrap().calc.last_recalc_cells
    );
}

//! Regressão: mesma Referencia em duas linhas do ORCAMENTO → mesmo Linha/Descricao/Preco.
#![allow(clippy::disallowed_methods)]

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::Session;
use serde_json::json;

fn xlsx_path() -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        r"c:\Users\Robson\source\grid-excel\LISTA REDUZIDA.xlsx",
        r"c:\Users\Robson\source\grid-excel\gridcraft\LISTA REDUZIDA.xlsx",
    ];
    CANDIDATES
        .iter()
        .map(std::path::Path::new)
        .find(|p| p.exists())
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

#[test]
fn two_mb006_rows_same_linha_and_descricao() {
    let Some(path) = xlsx_path() else {
        eprintln!("skip: LISTA REDUZIDA.xlsx not found");
        return;
    };
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    s.execute("sheet.activate", json!({"name": "COTACAO"})).unwrap();

    // Inspect stored Linha formula (names / nested LET) for case quirks.
    let l21 = s.execute("cell.get", json!({"cell": "L21"})).unwrap();
    eprintln!("L21 formula: {}", l21["formula"].as_str().unwrap_or(""));

    s.execute("cell.set", json!({"cell": "D16", "input": "MB006"})).unwrap();
    s.execute("cell.set", json!({"cell": "D21", "input": "MB006"})).unwrap();

    let l16 = num(&s, "L16");
    let l21n = num(&s, "L21");
    let e16 = text(&s, "E16");
    let e21 = text(&s, "E21");

    eprintln!("D16/D21=MB006 → L16={l16} L21={l21n}");
    eprintln!("E16={e16:?} E21={e21:?}");

    assert!(l16 > 0.0, "L16 should be a positive match index, got {l16}");
    assert_eq!(l16, l21n, "same Referencia must yield same Linha (L16={l16} L21={l21n})");
    assert_eq!(e16, e21, "same Linha must yield same Descricao");
    assert_eq!(cell(&s, "J16"), cell(&s, "J21"), "same Linha must yield same Preco_Unit");
}

/// Stale-path stress: row 21 was CT103 (Caterpillar); both rows then set to MB006 must converge.
#[test]
fn mb006_after_ct103_stale_row_converges() {
    let Some(path) = xlsx_path() else {
        return;
    };
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    s.execute("sheet.activate", json!({"name": "COTACAO"})).unwrap();

    s.execute("cell.set", json!({"cell": "D21", "input": "CT103"})).unwrap();
    let desc_ct = text(&s, "E21");
    eprintln!("after CT103: L21={} E21={desc_ct:?}", num(&s, "L21"));
    assert!(
        desc_ct.contains("CATERPILLAR") || desc_ct.contains("C7"),
        "CT103 should resolve to Caterpillar, got {desc_ct:?}"
    );

    s.execute("cell.set", json!({"cell": "D16", "input": "MB006"})).unwrap();
    s.execute("cell.set", json!({"cell": "D21", "input": "MB006"})).unwrap();

    let l16 = num(&s, "L16");
    let l21 = num(&s, "L21");
    let e16 = text(&s, "E16");
    let e21 = text(&s, "E21");
    eprintln!("both MB006: L16={l16} L21={l21} E16={e16:?} E21={e21:?}");

    assert_eq!(l16, l21);
    assert_eq!(e16, e21);
    assert!(!e21.contains("CATERPILLAR"), "MB006 must not keep stale Caterpillar desc: {e21:?}");
}

/// Com `recalc_pending` (cascata adiada da UI), editar Referencia não pode deixar Descricao/Linha
/// da mesma linha no lookup anterior (sintoma do screenshot: MB006 + Caterpillar).
/// `recalc_keys` sincroniza as colunas calculadas tocadas na hora; o job completo ainda roda depois.
#[test]
fn pending_recalc_same_row_syncs_before_job() {
    let Some(path) = xlsx_path() else {
        return;
    };
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    s.execute("sheet.activate", json!({"name": "COTACAO"})).unwrap();

    s.execute("cell.set", json!({"cell": "D21", "input": "CT103"})).unwrap();
    assert!(
        text(&s, "E21").contains("CATERPILLAR") || text(&s, "E21").contains("C7"),
        "setup CT103"
    );
    s.execute("cell.set", json!({"cell": "D16", "input": "MB006"})).unwrap();
    assert!(text(&s, "E16").contains("OM 352"), "D16 MB006 must resolve inline");

    // Arm unpaid-cascade window (big-book UI). LISTA REDUZIDA is below CASCADE_JOB_CELLS, so
    // we force pending like a prior deferred edit.
    {
        let d = s.doc_mut().unwrap();
        d.recalc_pending = true;
        d.recalc_cascade = None;
    }
    s.execute("cell.set", json!({"cell": "D21", "input": "MB006"})).unwrap();

    assert_eq!(text(&s, "D21"), "MB006");
    let l16 = num(&s, "L16");
    let l21 = num(&s, "L21");
    let e16 = text(&s, "E16");
    let e21 = text(&s, "E21");
    eprintln!("pending+sync: L16={l16} L21={l21} E16={e16:?} E21={e21:?}");
    assert!(s.doc().unwrap().recalc_pending, "full cascade still owed");
    assert_eq!(l16, l21, "same-row sync must refresh Linha before the background job");
    assert_eq!(e16, e21);
    assert!(!e21.contains("CATERPILLAR"), "must not keep stale Caterpillar after MB006: {e21:?}");
}

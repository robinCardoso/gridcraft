//! LISTA TESTE V3: how many formulas still need eval after open?
#![allow(clippy::disallowed_methods)]

use gridcraft_engine::Session;
use serde_json::json;

#[test]
fn lista_v3_open_formula_coverage() {
    let path = r"c:\Users\Robson\source\grid-excel\LISTA TESTE V3.xlsx";
    if !std::path::Path::new(path).exists() {
        eprintln!("skip: {path}");
        return;
    }
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path})).unwrap();
    let d = s.doc().unwrap();
    let mut need = 0usize;
    let mut total = 0usize;
    let mut need_active = 0usize;
    let mut total_active = 0usize;
    let active = d.wb.active_sheet;
    for (si, sh) in d.wb.sheets.iter().enumerate() {
        for (_, c) in sh.cells.iter() {
            if c.formula.is_some() {
                total += 1;
                if si == active {
                    total_active += 1;
                }
                if c.needs_formula_eval() {
                    need += 1;
                    if si == active {
                        need_active += 1;
                    }
                }
            }
        }
    }
    eprintln!(
        "active={active} ({}) formulas total={total} need_eval={need} active_need={need_active}/{total_active} graph={} recalc_pending={}",
        d.wb.active().map(|s| s.name.as_str()).unwrap_or("?"),
        d.calc.graph.len(),
        d.recalc_pending
    );
    assert!(d.recalc_pending, "a large workbook must owe a full recalculation after the scoped load pass");
    assert!(need > 1_000, "expected most formulas to still need eval on this file, got {need}");
    // Sample a known calculated column on LISTA_PRECO if still empty
    if let Some(i) = d.wb.sheets.iter().position(|s| s.name == "LISTA_PRECO") {
        s.execute("sheet.activate", json!({"index": i})).unwrap();
        let f3 = s.execute("cell.get", json!({"cell": "F3"})).unwrap();
        eprintln!("LISTA_PRECO F3 after activate: value={:?}", f3["value"]);
    }
}

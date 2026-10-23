// Local machine-only benchmark (times a real workbook on this desktop); the wasm clock lint
// doesn't apply: this test target never builds in the web app.
#![allow(clippy::disallowed_methods)]

use std::time::Instant;

#[test]
fn test_inspect_cotacao_table() {
    let reduced = r"c:\Users\Robson-PC\.antigravity\projetos\grif-excel\LISTA REDUZIDA.xlsx";
    let full = r"c:\Users\Robson-PC\.antigravity\projetos\grif-excel\LISTA TESTE V3.xlsx";
    let path = if std::path::Path::new(reduced).exists() {
        reduced
    } else if std::path::Path::new(full).exists() {
        full
    } else {
        println!("Test file not found, skipping benchmark test");
        return;
    };
    println!("Running benchmark with file: {path}");
    let bytes = std::fs::read(path).expect("read file");
    let t_read = Instant::now();
    let (mut wb, _report) = gridcraft_xlsx::read_xlsx(&bytes).expect("parse xlsx");
    println!("read_xlsx took {:.2?}", t_read.elapsed());

    println!("--- INSPECT COTACAO SHEET (SHEET 0) ---");
    if let Some(sh0) = wb.sheet(0) {
        println!("Sheet 0 name: {}", sh0.name);
        println!("Sheet 0 table count: {}", sh0.tables.len());
        for t in &sh0.tables {
            println!("  Table name: {}, range: {:?}", t.name, t.range);
            for (ci, c) in t.columns.iter().enumerate() {
                println!("    Col {}: name={}, formula={:?}", ci, c.name, c.formula);
            }
        }
        for r in [14, 15, 16] {
            // Excel rows 15, 16, 17
            for c in 0..12 {
                let cell_ref = gridcraft_core::CellRef::new(r, c);
                let cell = sh0.cell(cell_ref);
                let f_str = cell.and_then(|x| x.formula.as_ref()).map(|f| &f.text);
                let v_str = cell.map(|x| &x.value);
                println!("  Row {}, Col {} ({}): formula={:?}, val={:?}", r + 1, c, cell_ref.a1(), f_str, v_str);
            }
        }
    }

    println!("Inspect sheet 1 (LISTA_PRECO) before recalc:");
    if let Some(sh1) = wb.sheet(1) {
        println!("Sheet 1 name: {}", sh1.name);
        println!("Sheet 1 table count: {}", sh1.tables.len());
        for t in &sh1.tables {
            println!("  Table name: {}, range: {:?}", t.name, t.range);
            for (ci, c) in t.columns.iter().enumerate() {
                if ci < 10 {
                    println!("    Col {}: name={}, formula={:?}", ci, c.name, c.formula);
                }
            }
        }
        for r in 1..=3 {
            for c in 0..10 {
                let cell_ref = gridcraft_core::CellRef::new(r, c);
                let cell = sh1.cell(cell_ref);
                let f_str = cell.and_then(|x| x.formula.as_ref()).map(|f| &f.text);
                let v_str = cell.map(|x| &x.value);
                println!("  Row {}, Col {} ({}): formula={:?}, val={:?}", r + 1, c, cell_ref.a1(), f_str, v_str);
            }
        }
    }

    println!("Initializing calc with recalc_on_load:");
    let mut calc = gridcraft_calc::Calc::new();
    let t0 = Instant::now();
    calc.recalc_on_load(&mut wb);
    println!("  recalc_on_load took {:.2?}", t0.elapsed());

    println!("--- DETAILED EVAL OF F3 AND I3 ---");
    let sh1_idx = 1;
    let f3_ref = gridcraft_core::CellRef::new(2, 5);

    // Test 1: LISTA_PRECO[@Referencia]
    let v1 = gridcraft_calc::evaluate(&wb, sh1_idx, f3_ref, "LISTA_PRECO[@Referencia]");
    println!("Eval LISTA_PRECO[@Referencia] at F3: {:?}", v1);

    // Test 2: LISTA_PRECO[@cst_produto]
    let v2 = gridcraft_calc::evaluate(&wb, sh1_idx, f3_ref, "LISTA_PRECO[@cst_produto]");
    println!("Eval LISTA_PRECO[@cst_produto] at F3: {:?}", v2);

    // Test 3: PRECO_LIQUIDO[[#All],[Fornecedor]]
    let v3 = gridcraft_calc::evaluate(&wb, sh1_idx, f3_ref, "PRECO_LIQUIDO[[#All],[Fornecedor]]");
    println!(
        "Eval PRECO_LIQUIDO[[#All],[Fornecedor]]: {:?}",
        match &v3 {
            gridcraft_core::Value::Array(a) => format!("Array of {} elements", a.data.len()),
            other => format!("{:?}", other),
        }
    );

    // Test 4: REGRA_CST[]
    let v4 = gridcraft_calc::evaluate(&wb, sh1_idx, f3_ref, "REGRA_CST[]");
    println!(
        "Eval REGRA_CST[]: {:?}",
        match &v4 {
            gridcraft_core::Value::Array(a) => format!("Array of {} elements", a.data.len()),
            other => format!("{:?}", other),
        }
    );

    // Test 5: Full F3 formula
    let f3_cell = wb.sheet(sh1_idx).unwrap().cell(f3_ref);
    if let Some(f) = f3_cell.and_then(|c| c.formula.as_ref()) {
        println!("F3 Formula text: {}", f.text);
        let res = gridcraft_calc::evaluate(&wb, sh1_idx, f3_ref, &f.text);
        println!("F3 Eval result: {:?}", res);
    }

    let max_row = wb.sheet(1).and_then(|s| s.used_range()).map(|r| r.end.row).unwrap_or(100);
    let sample_rows = 102.min(max_row + 1);
    println!("Timing rows 2..{sample_rows} of Col F evaluation:");
    let t_col = Instant::now();
    for r in 2..sample_rows {
        let cell_ref = gridcraft_core::CellRef::new(r, 5);
        if let Some(f) = wb.sheet(1).unwrap().cell(cell_ref).and_then(|c| c.formula.as_ref()) {
            gridcraft_calc::evaluate(&wb, 1, cell_ref, &f.text);
        }
    }
    println!("  Rows 2..{sample_rows} of Col F took {:.2?}", t_col.elapsed());
    println!("Testing DocState and wb_switch_sheet(1) to LISTA_PRECO:");
    let mut doc = gridcraft_engine::DocState::new(wb, None, "Test".into());
    let t_switch = Instant::now();
    doc.wb_switch_sheet(1);
    println!("  wb_switch_sheet(1) took {:.2?}", t_switch.elapsed());

    println!("Checking cells in LISTA_PRECO after wb_switch_sheet(1):");
    if let Some(sh1) = doc.wb.sheet(1) {
        for r in 1..=4 {
            for c in [0, 1, 5, 6, 8] {
                let cell_ref = gridcraft_core::CellRef::new(r, c);
                let cell = sh1.cell(cell_ref);
                let f_str = cell.and_then(|x| x.formula.as_ref()).map(|f| &f.text);
                let v_str = cell.map(|x| &x.value);
                println!("  Row {}, Col {} ({}): formula={:?}, val={:?}", r + 1, c, cell_ref.a1(), f_str, v_str);
            }
        }
        // Assert that row 3 (index 2) calculated accurately
        let f3_val = sh1.value(gridcraft_core::CellRef::new(2, 5));
        assert_eq!(f3_val, gridcraft_core::Value::text("Bruto"));
        let i3_val = sh1.value(gridcraft_core::CellRef::new(2, 8));
        assert_eq!(i3_val, gridcraft_core::Value::text("NACIONAL"));
    }
}

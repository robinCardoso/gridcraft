use std::time::Instant;

#[test]
fn test_inspect_cotacao_table() {
    let path = r"c:\Users\Robson-PC\.antigravity\projetos\grif-excel\LISTA TESTE V3.xlsx";
    if !std::path::Path::new(path).exists() {
        println!("File {path} not found, skipping benchmark test");
        return;
    }
    let bytes = std::fs::read(path).expect("read file");
    let (mut wb, _report) = gridcraft_xlsx::read_xlsx(&bytes).expect("parse xlsx");

    let sh0 = wb.sheet_mut(0).unwrap();
    sh0.set_cell(gridcraft_core::CellRef::new(0, 1), gridcraft_model::Cell {
        value: gridcraft_core::Value::text("sc"),
        formula: None,
        style: gridcraft_model::StyleId(0),
    });

    println!("Initializing calc with recalc_on_load:");
    let mut calc = gridcraft_calc::Calc::new();
    let t0 = Instant::now();
    calc.recalc_on_load(&mut wb);
    println!("  recalc_on_load took {:.2?}", t0.elapsed());

    println!("Testing calc.cells_changed on changing B1 to 'sc':");
    let t1 = Instant::now();
    calc.cells_changed(&mut wb, &[(0, gridcraft_core::CellRef::new(0, 1))]);
    println!("  calc.cells_changed B1 took {:.2?}", t1.elapsed());

    println!("Testing calc.cells_changed on changing B2 to 'BA':");
    let sh0 = wb.sheet_mut(0).unwrap();
    sh0.set_cell(gridcraft_core::CellRef::new(1, 1), gridcraft_model::Cell {
        value: gridcraft_core::Value::text("BA"),
        formula: None,
        style: gridcraft_model::StyleId(0),
    });
    let t2 = Instant::now();
    calc.cells_changed(&mut wb, &[(0, gridcraft_core::CellRef::new(1, 1))]);
    println!("  calc.cells_changed B2 took {:.2?}", t2.elapsed());
}

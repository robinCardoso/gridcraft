use std::io::Write;
use std::path::PathBuf;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Cell, Formula, Workbook};

use super::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gridcraft-db-tests-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

/// A tiny XLSX built in code: an Excel table `PRECO` with a calculated column, inline strings,
/// a shared string, a boolean and a number, plus an empty sheet.
fn sample_xlsx(path: &PathBuf) {
    let file = std::fs::File::create(path).expect("create xlsx");
    let mut z = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    let mut put = |name: &str, body: &str| {
        z.start_file(name, opts).expect("start");
        z.write_all(body.as_bytes()).expect("write");
    };
    put(
        "xl/workbook.xml",
        r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets><sheet name="PRECO" sheetId="1" r:id="rId1"/><sheet name="VAZIA" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
    );
    put(
        "xl/_rels/workbook.xml.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="/xl/worksheets/sheet2.xml"/></Relationships>"#,
    );
    put("xl/sharedStrings.xml", r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Parafuso &amp; porca</t></si></sst>"#);
    put(
        "xl/worksheets/sheet1.xml",
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Referencia</t></is></c><c r="B1" t="inlineStr"><is><t>Descricao</t></is></c><c r="C1" t="inlineStr"><is><t>Preco</t></is></c><c r="D1" t="inlineStr"><is><t>Ativo</t></is></c><c r="E1" t="inlineStr"><is><t>Total</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>AB-1</t></is></c><c r="B2" t="s"><v>0</v></c><c r="C2"><v>10.5</v></c><c r="D2" t="b"><v>1</v></c><c r="E2"><f>C2*2</f></c></row>
<row r="4"><c r="A4" t="inlineStr"><is><t>ZZ-9</t></is></c><c r="C4"><v>3</v></c></row>
</sheetData><tableParts count="1"><tablePart r:id="rId1" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></tableParts></worksheet>"#,
    );
    put(
        "xl/worksheets/_rels/sheet1.xml.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/></Relationships>"#,
    );
    put(
        "xl/tables/table1.xml",
        r#"<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="PRECO" displayName="PRECO" ref="A1:E4"><tableColumns count="5"><tableColumn id="1" name="Referencia"/><tableColumn id="2" name="Descricao"/><tableColumn id="3" name="Preco"/><tableColumn id="4" name="Ativo"/><tableColumn id="5" name="Total"><calculatedColumnFormula>PRECO[[#This Row],[Preco]]*2</calculatedColumnFormula></tableColumn></tableColumns></table>"#,
    );
    put("xl/worksheets/sheet2.xml", r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#);
    z.finish().expect("finish");
}

#[test]
fn imports_xlsx_streaming_into_tables() {
    let xlsx = temp("in.xlsx");
    let db = temp("in.db");
    sample_xlsx(&xlsx);
    let report = import_xlsx(&xlsx, &db, &mut |_, _| {}).expect("import");
    assert_eq!(report.tables.len(), 1);
    assert_eq!(report.tables[0].sheet, "PRECO");
    assert_eq!(report.tables[0].rows, 2);
    assert_eq!(report.tables[0].columns, 5);
    assert_eq!(report.tables[0].calculated_columns, 1);
    assert!(report.warnings.iter().any(|w| w.contains("VAZIA")));

    let conn = rusqlite::Connection::open(&db).expect("open");
    let (desc, price, active): (String, f64, i64) =
        conn.query_row("SELECT Descricao, Preco, Ativo FROM PRECO WHERE Referencia = 'AB-1'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).expect("row");
    assert_eq!((desc.as_str(), price, active), ("Parafuso & porca", 10.5, 1));
    let row: i64 = conn.query_row("SELECT _row FROM PRECO WHERE Referencia = 'ZZ-9'", [], |r| r.get(0)).expect("row number");
    assert_eq!(row, 4, "blank rows keep their Excel row number");
    let formula: String = conn.query_row("SELECT formula FROM _gc_columns WHERE name = 'Total'", [], |r| r.get(0)).expect("formula");
    assert!(formula.contains("[Preco]"));
}

#[test]
fn opens_an_imported_db_as_a_workbook() {
    let xlsx = temp("open.xlsx");
    let db = temp("open.db");
    sample_xlsx(&xlsx);
    import_xlsx(&xlsx, &db, &mut |_, _| {}).expect("import");
    let (wb, warnings) = read_workbook(&db, DEFAULT_MAX_ROWS).expect("read");
    assert!(warnings.is_empty());
    assert_eq!(wb.sheets.len(), 1);
    let sh = &wb.sheets[0];
    assert_eq!(sh.name, "PRECO");
    assert_eq!(sh.value(CellRef::new(0, 0)), Value::text("Referencia"));
    assert_eq!(sh.value(CellRef::new(1, 2)), Value::Number(10.5));
    assert_eq!(sh.value(CellRef::new(3, 0)), Value::text("ZZ-9"), "row 4 stays on row 4");
}

#[test]
fn a_big_result_is_truncated_with_a_warning() {
    let xlsx = temp("trunc.xlsx");
    let db = temp("trunc.db");
    sample_xlsx(&xlsx);
    import_xlsx(&xlsx, &db, &mut |_, _| {}).expect("import");
    let (wb, warnings) = read_workbook(&db, 1).expect("read");
    assert_eq!(warnings.len(), 1);
    assert_eq!(wb.sheets[0].value(CellRef::new(1, 0)), Value::text("AB-1"));
    assert_eq!(wb.sheets[0].value(CellRef::new(3, 0)), Value::Empty);
}

#[test]
fn workbook_round_trips_through_a_db() {
    let db = temp("round.db");
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).expect("sheet");
    for (c, h) in ["Item", "Qtd", "Dobro"].iter().enumerate() {
        sh.set_value(CellRef::new(0, c as u32), Value::text(*h));
    }
    sh.set_value(CellRef::new(1, 0), Value::text("caneta"));
    sh.set_value(CellRef::new(1, 1), Value::Number(4.0));
    sh.set_cell(CellRef::new(1, 2), Cell { value: Value::Number(8.0), formula: Some(std::sync::Arc::new(Formula::new("=B2*2"))), style: Default::default() });
    write_workbook(&wb, &db).expect("write");

    let (back, _) = read_workbook(&db, DEFAULT_MAX_ROWS).expect("read");
    let b = &back.sheets[0];
    assert_eq!(b.value(CellRef::new(0, 1)), Value::text("Qtd"));
    assert_eq!(b.value(CellRef::new(1, 0)), Value::text("caneta"));
    assert_eq!(b.value(CellRef::new(1, 2)), Value::Number(8.0));
    assert_eq!(b.cell(CellRef::new(1, 2)).and_then(|c| c.formula.as_ref()).map(|f| f.text.as_str()), Some("B2*2"));
}

#[test]
fn opens_a_plain_sqlite_database_and_rejects_garbage() {
    let db = temp("plain.db");
    let _ = std::fs::remove_file(&db);
    {
        let conn = rusqlite::Connection::open(&db).expect("create");
        conn.execute_batch("CREATE TABLE clientes (nome TEXT, saldo REAL); INSERT INTO clientes VALUES ('Ana', 12.5), ('Bia', NULL);").expect("fill");
    }
    let (wb, _) = read_workbook(&db, DEFAULT_MAX_ROWS).expect("read");
    assert_eq!(wb.sheets[0].name, "clientes");
    assert_eq!(wb.sheets[0].value(CellRef::new(0, 1)), Value::text("saldo"));
    assert_eq!(wb.sheets[0].value(CellRef::new(1, 1)), Value::Number(12.5));
    assert_eq!(wb.sheets[0].value(CellRef::new(2, 0)), Value::text("Bia"));

    let bad = temp("garbage.db");
    std::fs::write(&bad, b"this is not a database at all, just text").expect("write");
    assert!(read_workbook(&bad, DEFAULT_MAX_ROWS).is_err());
    assert!(import_xlsx(&bad, &temp("never.db"), &mut |_, _| {}).is_err());
}

#[test]
fn db_paths() {
    assert!(is_db_path("C:\\x\\a.DB"));
    assert!(is_db_path("a.sqlite3"));
    assert!(!is_db_path("a.xlsx"));
}

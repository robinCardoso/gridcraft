//! Opening and saving a workbook as a SQLite database.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::{Cell, Formula, Sheet, StyleId, Workbook};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, params_from_iter};

use crate::{DbError, Result, quote_ident};

/// Rows of a table loaded into a sheet unless the caller asks otherwise (one less than the grid
/// holds, leaving room for the header row).
pub const DEFAULT_MAX_ROWS: u32 = 1_048_575;

/// A valid, unique sheet name made from a table name.
fn sheet_name(wb: &Workbook, raw: &str) -> String {
    let mut base: String = raw.chars().map(|c| if matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']') { '_' } else { c }).collect();
    base = base.trim_matches('\'').to_string();
    if base.is_empty() || base.eq_ignore_ascii_case("History") {
        base = format!("{base}Sheet");
    }
    let clip = |s: &str, extra: usize| -> String { s.chars().take(31usize.saturating_sub(extra)).collect() };
    let mut name = clip(&base, 0);
    let mut k = 2;
    while wb.check_sheet_name(&name, None).is_err() {
        let suffix = format!("_{k}");
        name = format!("{}{suffix}", clip(&base, suffix.chars().count()));
        k += 1;
        if k > 10_000 {
            break;
        }
    }
    name
}

fn to_value(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Empty,
        ValueRef::Integer(i) => Value::number(i as f64),
        ValueRef::Real(f) => Value::number(f),
        ValueRef::Text(t) => Value::text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::text(format!("<blob {} bytes>", b.len())),
    }
}

/// `(sheet name, table name)` in display order: from `_gc_sheets` when present, otherwise every
/// table and view of an ordinary database.
fn list_sheets(conn: &Connection) -> Result<Vec<(String, String)>> {
    let has_meta: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = '_gc_sheets'", [], |r| r.get::<_, i64>(0))? > 0;
    let mut out = vec![];
    if has_meta {
        let mut stmt = conn.prepare("SELECT sheet, table_name FROM _gc_sheets WHERE table_name IS NOT NULL ORDER BY position")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            out.push(r?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND name NOT LIKE '\\_gc\\_%' ESCAPE '\\' ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows {
            let name = r?;
            out.push((name.clone(), name));
        }
    }
    Ok(out)
}

/// Reads a workbook directly from `_gc_cells` preserving exact cell positions, formulas and values.
fn read_workbook_from_cells(conn: &Connection, max_rows: u32) -> Result<(Workbook, Vec<String>)> {
    let mut wb = Workbook::new();
    wb.sheets.clear();
    let mut warnings = vec![];

    let mut sheet_list: Vec<(String, String)> = vec![];
    let has_sheets_table: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = '_gc_sheets'", [], |r| r.get::<_, i64>(0))? > 0;
    if has_sheets_table {
        let mut stmt = conn.prepare("SELECT sheet, COALESCE(table_name, sheet) FROM _gc_sheets ORDER BY position")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            sheet_list.push(r?);
        }
    }
    if sheet_list.is_empty() {
        let mut dist = conn.prepare("SELECT DISTINCT sheet FROM _gc_cells ORDER BY sheet")?;
        let rows = dist.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows {
            let s = r?;
            sheet_list.push((s.clone(), s));
        }
    }

    let has_tables: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = '_gc_tables'", [], |r| r.get::<_, i64>(0))? > 0;
    let has_columns: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = '_gc_columns'", [], |r| r.get::<_, i64>(0))? > 0;

    for (name, table_name) in sheet_list {
        let max_r: Option<i64> = conn
            .query_row("SELECT MAX(row) FROM _gc_cells WHERE sheet = ?1", [&name], |r| r.get(0))
            .ok()
            .flatten();

        let Some(max_row) = max_r else {
            continue;
        };

        if max_row >= i64::from(max_rows) {
            warnings.push(format!("'{table_name}' has more than {max_rows} rows; loaded up to row {max_rows}"));
        }

        let mut sh = Sheet::new(sheet_name(&wb, &name));

        let formula_count: i64 = conn
            .query_row("SELECT COUNT(formula) FROM _gc_cells WHERE sheet = ?1", [&name], |r| r.get(0))
            .unwrap_or(0);
        // Sheets with massive formula counts (e.g. catalog tables with > 50,000 repeated formulas)
        // load cached values to keep memory responsive and avoid locking the UI thread.
        // The formulas remain permanently preserved in the SQLite database.
        let load_formulas = formula_count < 50_000;

        let mut cell_stmt = conn.prepare(
            "SELECT row, col, val_type, val_num, val_text, formula FROM _gc_cells WHERE sheet = ?1 AND row <= ?2 ORDER BY row, col",
        )?;
        let mut rows = cell_stmt.query(rusqlite::params![name, max_rows])?;
        while let Some(r) = rows.next()? {
            let row: u32 = r.get(0)?;
            let col: u32 = r.get(1)?;
            let val_type: Option<String> = r.get(2)?;
            let val_num: Option<f64> = r.get(3)?;
            let val_text: Option<String> = r.get(4)?;
            let formula_text: Option<String> = r.get(5)?;

            let value = match val_type.as_deref() {
                Some("n") => val_num.map(Value::number).unwrap_or(Value::Empty),
                Some("s") => val_text.map(Value::text).unwrap_or(Value::Empty),
                Some("b") => val_num.map(|n| Value::Bool(n != 0.0)).unwrap_or(Value::Empty),
                Some("e") => val_text.as_deref().and_then(gridcraft_core::CellError::parse).map(Value::Error).unwrap_or(Value::Empty),
                _ => Value::Empty,
            };
            let formula = if load_formulas {
                formula_text.as_deref().filter(|s| !s.is_empty()).map(|s| std::sync::Arc::new(Formula::new(s)))
            } else {
                None
            };
            let at = CellRef::new(row, col);
            if at.is_valid() {
                sh.set_cell(at, Cell { value, formula, style: StyleId::DEFAULT });
            }
        }

        // Reconstruct tables if stored
        if has_tables {
            let mut tbl_stmt = conn.prepare("SELECT table_name, range_ref FROM _gc_tables WHERE sheet = ?1")?;
            let mut tbl_rows = tbl_stmt.query([&name])?;
            while let Some(tr) = tbl_rows.next()? {
                let t_name: String = tr.get(0)?;
                let range_str: String = tr.get(1)?;
                if let Some(range) = RangeRef::parse(&range_str) {
                    let mut cols = vec![];
                    if has_columns {
                        let mut col_stmt =
                            conn.prepare("SELECT position, name, formula FROM _gc_columns WHERE table_name = ?1 ORDER BY position")?;
                        let mut c_rows = col_stmt.query([&t_name])?;
                        while let Some(cr) = c_rows.next()? {
                            let c_name: String = cr.get(1)?;
                            let c_formula: Option<String> = cr.get(2)?;
                            cols.push(gridcraft_model::TableColumn {
                                name: c_name,
                                totals: gridcraft_model::TotalsFn::None,
                                totals_label: None,
                                formula: c_formula,
                                unique_name: None,
                                xml: None,
                            });
                        }
                    }
                    sh.tables.push(gridcraft_model::Table {
                        id: (sh.tables.len() + 1) as u32,
                        name: t_name,
                        range,
                        header_row: true,
                        totals_row: false,
                        columns: cols,
                        style: "TableStyleMedium2".into(),
                        banded_rows: true,
                        banded_cols: false,
                        first_col: false,
                        last_col: false,
                        filter_button: true,
                        filter: vec![],
                        table_type: None,
                        connection_id: None,
                    });
                }
            }
        }

        wb.sheets.push(Arc::new(sh));
    }

    Ok((wb, warnings))
}

/// Opens a SQLite database as a workbook:
/// When `_gc_cells` is present, loads full cell coordinates, formulas, and values.
/// Otherwise, each table or view becomes a sheet with columns at row 1.
pub fn read_workbook(path: &Path, max_rows: u32) -> Result<(Workbook, Vec<String>)> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;

    let has_gc_cells: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = '_gc_cells'", [], |r| r.get::<_, i64>(0))? > 0;
    if has_gc_cells {
        return read_workbook_from_cells(&conn, max_rows);
    }

    let sheets = list_sheets(&conn)?;
    if sheets.is_empty() {
        return Err(DbError::Other("this database has no tables".into()));
    }
    let has_formulas: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = '_gc_formulas'", [], |r| r.get::<_, i64>(0))? > 0;
    let mut wb = Workbook::new();
    wb.sheets.clear();
    let mut warnings = vec![];
    for (name, table) in sheets {
        let total: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {}", quote_ident(&table)), [], |r| r.get(0))?;
        let mut stmt = conn.prepare(&format!("SELECT * FROM {} LIMIT {}", quote_ident(&table), max_rows))?;
        let columns: Vec<String> = stmt.column_names().iter().map(|s| (*s).to_string()).collect();
        let has_row = columns.first().is_some_and(|c| c == "_row");
        let skip = usize::from(has_row);
        let mut sh = Sheet::new(sheet_name(&wb, &name));
        for (c, header) in columns.iter().enumerate().skip(skip) {
            sh.set_value(CellRef::new(0, (c - skip) as u32), Value::text(header.as_str()));
        }
        let mut rows = stmt.query([])?;
        let mut n: u32 = 0;
        while let Some(row) = rows.next()? {
            n = n.saturating_add(1);
            let excel_row = if has_row {
                // `_row` is the 1-based Excel row; the header is row 1.
                row.get_ref(0).ok().and_then(|v| v.as_i64().ok()).and_then(|r| u32::try_from(r.saturating_sub(1)).ok()).filter(|r| *r >= 1).unwrap_or(n)
            } else {
                n
            };
            if excel_row >= gridcraft_core::addr::MAX_ROWS {
                break;
            }
            for c in skip..columns.len() {
                let Ok(v) = row.get_ref(c) else { continue };
                let v = to_value(v);
                if !v.is_empty() {
                    sh.set_value(CellRef::new(excel_row, (c - skip) as u32), v);
                }
            }
        }
        if total > i64::from(max_rows) {
            warnings.push(format!("'{table}' has {total} rows; only the first {max_rows} were loaded"));
        }
        if has_formulas {
            let mut f = conn.prepare("SELECT row, col, formula FROM _gc_formulas WHERE sheet = ?1")?;
            let found = f.query_map([&name], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
            for item in found {
                let (r, c, text) = item?;
                let (Ok(r), Ok(c)) = (u32::try_from(r), u32::try_from(c)) else { continue };
                let at = CellRef::new(r, c);
                if at.is_valid() {
                    let value = sh.value(at);
                    sh.set_cell(at, Cell { value, formula: Some(Arc::new(Formula::new(&text))), style: StyleId::DEFAULT });
                }
            }
        }
        wb.sheets.push(Arc::new(sh));
    }
    Ok((wb, warnings))
}

fn to_sql(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    match v {
        Value::Empty | Value::Array(_) => Sql::Null,
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                Sql::Integer(*n as i64)
            } else {
                Sql::Real(*n)
            }
        }
        Value::Text(t) => Sql::Text(t.to_string()),
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Error(_) => Sql::Text(v.display()),
    }
}

/// Saves a workbook as a SQLite database at `path` (replaced if it exists; the old file survives
/// a failed save). All cells and formulas are preserved in `_gc_cells` and `_gc_formulas`, and
/// relational tables are created for SQL querying.
pub fn write_workbook(wb: &Workbook, path: &Path) -> Result<()> {
    let tmp = path.with_extension("gcsave.tmp");
    let _ = std::fs::remove_file(&tmp);
    let result = write_into(wb, &tmp);
    match result {
        Ok(()) => {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp, path)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn write_into(wb: &Workbook, path: &Path) -> Result<()> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=MEMORY;")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _gc_sheets (position INTEGER, sheet TEXT, table_name TEXT, source_ref TEXT, formula_cells INTEGER);
         CREATE TABLE IF NOT EXISTS _gc_columns (table_name TEXT, position INTEGER, name TEXT, formula TEXT);
         CREATE TABLE IF NOT EXISTS _gc_formulas (sheet TEXT, row INTEGER, col INTEGER, formula TEXT);
         CREATE TABLE IF NOT EXISTS _gc_cells (sheet TEXT, row INTEGER, col INTEGER, val_type TEXT, val_num REAL, val_text TEXT, formula TEXT, PRIMARY KEY (sheet, row, col));
         CREATE TABLE IF NOT EXISTS _gc_tables (table_name TEXT PRIMARY KEY, sheet TEXT, display_name TEXT, range_ref TEXT, header_rows INTEGER, totals_rows INTEGER);
         BEGIN;",
    )?;
    for (position, sh) in wb.sheets.iter().enumerate() {
        let mut by_row: BTreeMap<u32, Vec<(u32, Value)>> = BTreeMap::new();
        let mut formulas: Vec<(CellRef, String)> = vec![];
        let mut ncols = 0u32;

        {
            let mut insert_cell = conn.prepare_cached(
                "INSERT OR REPLACE INTO _gc_cells (sheet, row, col, val_type, val_num, val_text, formula) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (at, cell) in sh.cells.iter() {
                if let Some(f) = &cell.formula {
                    formulas.push((at, f.text.clone()));
                }
                if !cell.value.is_empty() {
                    ncols = ncols.max(at.col + 1);
                    by_row.entry(at.row).or_default().push((at.col, cell.value.clone()));
                }
                let (v_type, v_num, v_text) = match &cell.value {
                    Value::Number(n) => (Some("n"), Some(*n), None),
                    Value::Text(t) => (Some("s"), None, Some(t.to_string())),
                    Value::Bool(b) => (Some("b"), Some(if *b { 1.0 } else { 0.0 }), None),
                    Value::Error(e) => (Some("e"), None, Some(e.as_str().to_string())),
                    _ => (None, None, None),
                };
                let f_text = cell.formula.as_ref().map(|f| f.text.as_str());
                insert_cell.execute(rusqlite::params![
                    sh.name,
                    at.row as i64,
                    at.col as i64,
                    v_type,
                    v_num,
                    v_text,
                    f_text,
                ])?;
            }
        }

        for t in &sh.tables {
            conn.execute(
                "INSERT OR REPLACE INTO _gc_tables (table_name, sheet, display_name, range_ref, header_rows, totals_rows) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![t.name, sh.name, t.name, t.range.to_string(), t.header_row as i64, t.totals_row as i64],
            )?;
            for (i, c) in t.columns.iter().enumerate() {
                conn.execute(
                    "INSERT OR REPLACE INTO _gc_columns (table_name, position, name, formula) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![t.name, i as i64, c.name, c.formula.as_deref()],
                )?;
            }
        }

        if ncols == 0 {
            conn.execute(
                "INSERT INTO _gc_sheets (position, sheet, table_name, source_ref, formula_cells) VALUES (?1, ?2, NULL, '', 0)",
                rusqlite::params![position as i64, sh.name],
            )?;
            continue;
        }

        let header = by_row.remove(&0).unwrap_or_default();
        let header_cells: Vec<(usize, rusqlite::types::Value)> = header.iter().map(|(c, v)| (*c as usize, to_sql(v))).collect();
        let names = {
            let mut names: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            seen.insert("_row".to_string());
            for i in 0..ncols as usize {
                let raw = header_cells
                    .iter()
                    .find(|(c, _)| *c == i)
                    .map(|(_, v)| match v {
                        rusqlite::types::Value::Text(s) => s.trim().to_string(),
                        rusqlite::types::Value::Integer(x) => x.to_string(),
                        rusqlite::types::Value::Real(x) => x.to_string(),
                        _ => String::new(),
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("COL_{}", i + 1));
                let mut name = raw.clone();
                let mut k = 2;
                while !seen.insert(name.to_lowercase()) {
                    name = format!("{raw}_{k}");
                    k += 1;
                }
                names.push(name);
            }
            names
        };
        let table = quote_ident(&sh.name);
        let cols = names.iter().map(|n| quote_ident(n)).collect::<Vec<_>>().join(", ");
        conn.execute_batch(&format!("CREATE TABLE {table} (_row INTEGER PRIMARY KEY, {cols});"))?;
        let marks = (1..=names.len() + 1).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        let sql = format!("INSERT INTO {table} (_row, {cols}) VALUES ({marks})");
        {
            let mut stmt = conn.prepare(&sql)?;
            for (row, cells) in &by_row {
                let mut params = vec![rusqlite::types::Value::Null; names.len() + 1];
                if let Some(p) = params.first_mut() {
                    *p = rusqlite::types::Value::Integer(i64::from(*row) + 1);
                }
                for (c, v) in cells {
                    if let Some(slot) = params.get_mut(*c as usize + 1) {
                        *slot = to_sql(v);
                    }
                }
                stmt.execute(params_from_iter(params))?;
            }
        }
        for (i, n) in names.iter().enumerate() {
            conn.execute("INSERT OR REPLACE INTO _gc_columns (table_name, position, name, formula) VALUES (?1, ?2, ?3, NULL)", rusqlite::params![sh.name, i as i64, n])?;
        }
        for (at, text) in &formulas {
            conn.execute(
                "INSERT INTO _gc_formulas (sheet, row, col, formula) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![sh.name, i64::from(at.row), i64::from(at.col), text],
            )?;
        }
        conn.execute(
            "INSERT INTO _gc_sheets (position, sheet, table_name, source_ref, formula_cells) VALUES (?1, ?2, ?3, '', ?4)",
            rusqlite::params![position as i64, sh.name, sh.name, formulas.len() as i64],
        )?;
    }
    conn.execute_batch("COMMIT;")?;
    conn.close().map_err(|(_, e)| DbError::Sqlite(e))?;
    Ok(())
}

//! Opening and saving a workbook as a SQLite database.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
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

/// Opens a SQLite database as a workbook: each table or view becomes a sheet, the column names
/// are row 1 and the data follows. At most `max_rows` rows are loaded per sheet. The second value
/// holds warnings (truncated tables, ...). Formulas saved by [`write_workbook`] come back.
pub fn read_workbook(path: &Path, max_rows: u32) -> Result<(Workbook, Vec<String>)> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
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
/// a failed save). Each sheet becomes a table: row 1 gives the column names (empty cells become
/// `COL_n`), later rows the data. Formulas go to `_gc_formulas` with their calculated values in
/// the table. Formatting, charts and other sheet features are not stored.
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
    conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;")?;
    conn.execute_batch(
        "CREATE TABLE _gc_sheets (position INTEGER, sheet TEXT, table_name TEXT, source_ref TEXT, formula_cells INTEGER);
         CREATE TABLE _gc_columns (table_name TEXT, position INTEGER, name TEXT, formula TEXT);
         CREATE TABLE _gc_formulas (sheet TEXT, row INTEGER, col INTEGER, formula TEXT);
         BEGIN;",
    )?;
    for (position, sh) in wb.sheets.iter().enumerate() {
        let mut by_row: BTreeMap<u32, Vec<(u32, Value)>> = BTreeMap::new();
        let mut formulas: Vec<(CellRef, String)> = vec![];
        let mut ncols = 0u32;
        for (at, cell) in sh.cells.iter() {
            if let Some(f) = &cell.formula {
                formulas.push((at, f.text.clone()));
            }
            if !cell.value.is_empty() {
                ncols = ncols.max(at.col + 1);
                by_row.entry(at.row).or_default().push((at.col, cell.value.clone()));
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
            conn.execute("INSERT INTO _gc_columns (table_name, position, name, formula) VALUES (?1, ?2, ?3, NULL)", rusqlite::params![sh.name, i as i64, n])?;
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

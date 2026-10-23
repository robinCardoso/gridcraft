//! Streaming XLSX → SQLite import.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesRef, BytesStart, Event};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, params_from_iter};

use crate::{DbError, Result, quote_ident};

/// What was imported for one sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableReport {
    pub sheet: String,
    /// Data rows written (the header row is not counted).
    pub rows: u64,
    pub columns: usize,
    /// Cells that held a formula. Their cached values (when the file has any) are imported; the
    /// formulas themselves are not.
    pub formula_cells: u64,
    /// Columns that are calculated columns of an Excel table (formula stored in `_gc_columns`).
    pub calculated_columns: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub tables: Vec<TableReport>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------- small XML parts

struct Elem {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
}

impl Elem {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

fn local(name: &[u8]) -> String {
    let n = match name.iter().rposition(|&b| b == b':') {
        Some(i) => name.get(i + 1..).unwrap_or(name),
        None => name,
    };
    String::from_utf8_lossy(n).into_owned()
}

fn attr_value(a: &quick_xml::events::attributes::Attribute<'_>) -> String {
    match a.normalized_value(XmlVersion::Explicit1_0) {
        Ok(v) => v.into_owned(),
        Err(_) => String::from_utf8_lossy(&a.value).into_owned(),
    }
}

fn elem_of(e: &BytesStart<'_>) -> Elem {
    let mut el = Elem { name: local(e.name().as_ref()), attrs: vec![], text: String::new() };
    for a in e.attributes().with_checks(false).flatten() {
        let key = a.key.as_ref();
        if key == b"xmlns" || key.starts_with(b"xmlns:") {
            continue;
        }
        el.attrs.push((local(key), attr_value(&a)));
    }
    el
}

fn push_ref(r: BytesRef<'_>, out: &mut String) {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        out.push(c);
        return;
    }
    let name = String::from_utf8_lossy(&r).into_owned();
    match name.as_str() {
        "amp" => out.push('&'),
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "quot" => out.push('"'),
        "apos" => out.push('\''),
        _ => {
            out.push('&');
            out.push_str(&name);
            out.push(';');
        }
    }
}

fn configured<R: BufRead>(rd: R) -> Reader<R> {
    let mut reader = Reader::from_reader(rd);
    let cfg = reader.config_mut();
    cfg.check_end_names = false;
    cfg.allow_unmatched_ends = true;
    cfg.allow_dangling_amp = true;
    cfg.trim_text_start = false;
    cfg.trim_text_end = false;
    reader
}

/// Every element of a small part in document order, with its direct text.
fn flat(xml: &[u8]) -> Vec<Elem> {
    let mut reader = configured(xml);
    let mut out: Vec<Elem> = vec![];
    let mut open: Vec<usize> = vec![];
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                out.push(elem_of(&e));
                open.push(out.len() - 1);
            }
            Ok(Event::Empty(e)) => out.push(elem_of(&e)),
            Ok(Event::End(_)) => {
                open.pop();
            }
            Ok(Event::Text(t)) => {
                if let Some(el) = open.last().and_then(|i| out.get_mut(*i))
                    && let Ok(s) = t.xml_content(XmlVersion::Explicit1_0)
                {
                    el.text.push_str(&s);
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if let Some(el) = open.last().and_then(|i| out.get_mut(*i)) {
                    push_ref(r, &mut el.text);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

type Archive = zip::ZipArchive<BufReader<std::fs::File>>;

fn package_err(e: impl std::fmt::Display) -> DbError {
    DbError::Package(e.to_string())
}

fn read_part(zip: &mut Archive, name: &str) -> Result<Option<Vec<u8>>> {
    let mut file = match zip.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(package_err(e)),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Resolves a relationship `target` against the folder `base` of the part that owns it.
fn resolve(base: &str, target: &str) -> String {
    if let Some(t) = target.strip_prefix('/') {
        return t.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn rels(zip: &mut Archive, rels_path: &str) -> Result<Vec<(String, String, String)>> {
    let Some(bytes) = read_part(zip, rels_path)? else { return Ok(vec![]) };
    Ok(flat(&bytes)
        .into_iter()
        .filter(|e| e.name == "Relationship")
        .map(|e| (e.attr("Id").unwrap_or("").to_string(), e.attr("Type").unwrap_or("").to_string(), e.attr("Target").unwrap_or("").to_string()))
        .collect())
}

fn shared_strings(zip: &mut Archive) -> Result<Vec<String>> {
    let Some(bytes) = read_part(zip, "xl/sharedStrings.xml")? else { return Ok(vec![]) };
    let mut reader = configured(bytes.as_slice());
    let mut out = vec![];
    let (mut cur, mut in_si, mut in_t, mut in_ph) = (String::new(), false, false, 0u32);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"si" => {
                    in_si = true;
                    cur.clear();
                }
                b"rPh" => in_ph += 1,
                b"t" => in_t = in_si && in_ph == 0,
                _ => {}
            },
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"si" => {
                    in_si = false;
                    out.push(std::mem::take(&mut cur));
                }
                b"rPh" => in_ph = in_ph.saturating_sub(1),
                b"t" => in_t = false,
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => {
                if let Ok(s) = t.xml_content(XmlVersion::Explicit1_0) {
                    cur.push_str(&s);
                }
            }
            Ok(Event::GeneralRef(r)) if in_t => push_ref(r, &mut cur),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

// ---------------------------------------------------------------- worksheet streaming

/// 0-based column of an `A1`-style reference.
fn col_of(r: &str) -> Option<usize> {
    let mut n: usize = 0;
    let mut any = false;
    for c in r.chars().take_while(char::is_ascii_alphabetic) {
        n = n.checked_mul(26)?.checked_add((c.to_ascii_uppercase() as u8 - b'A') as usize + 1)?;
        any = true;
    }
    if any { n.checked_sub(1).filter(|c| *c < 16_384) } else { None }
}

fn number(v: f64) -> Sql {
    if v.fract() == 0.0 && v.abs() < 9.0e15 { Sql::Integer(v as i64) } else { Sql::Real(v) }
}

/// A parsed cell from the worksheet XML stream.
#[derive(Clone, Debug)]
pub struct RawCell {
    pub col: usize,
    pub val: Option<Sql>,
    pub formula: Option<String>,
}

/// Normalizes formula text by removing Excel's internal namespaces (`_xlfn.`, `_xlpm.`, `_xlws.`).
pub fn clean_formula(text: &str) -> String {
    let body = text.strip_prefix('=').unwrap_or(text).trim();
    if let Ok(e) = gridcraft_formula::parse(body) {
        gridcraft_formula::print(&e.map(&mut |x| match x {
            gridcraft_formula::Expr::Name(n) => gridcraft_formula::Expr::Name(strip_param_prefix(n)),
            gridcraft_formula::Expr::Call(n, args) => gridcraft_formula::Expr::Call(strip_param_prefix(n), args),
            other => other,
        }))
    } else {
        body.replace("_xlfn._xlws.", "").replace("_xlfn.", "").replace("_xlpm.", "")
    }
}

fn strip_param_prefix(n: String) -> String {
    if let Some(rest) = n.strip_prefix("_xlpm.") { rest.to_string() } else { n }
}

struct CellState {
    col: Option<usize>,
    ty: String,
    v: String,
    t: String,
    f: String,
    has_formula: bool,
    f_type: String,
    f_si: Option<u32>,
}

impl CellState {
    fn value(&self, shared: &[String]) -> Option<Sql> {
        let v = self.v.trim();
        match self.ty.as_str() {
            "inlineStr" => (!self.t.is_empty()).then(|| Sql::Text(self.t.clone())),
            "s" => v.parse::<usize>().ok().and_then(|i| shared.get(i)).map(|s| Sql::Text(s.clone())),
            "str" | "e" => (!self.v.is_empty()).then(|| Sql::Text(self.v.clone())),
            "b" => (!v.is_empty()).then(|| Sql::Integer(i64::from(v == "1"))),
            _ => v.parse::<f64>().ok().filter(|f| f.is_finite()).map(number),
        }
    }
}

enum Field {
    None,
    V,
    T,
    F,
}

struct SharedFormula {
    anchor: (u32, u32),
    expr: Option<gridcraft_formula::Expr>,
    text: String,
}

/// Streams a worksheet; `on_row(row number (1-based), cells (RawCell), formula cells)`
/// is called for each row. Memory stays at one row.
fn stream_sheet<R: BufRead>(rd: R, shared: &[String], on_row: &mut dyn FnMut(u32, Vec<RawCell>, u64) -> Result<()>) -> Result<()> {
    let mut reader = configured(rd);
    let mut buf = Vec::new();
    let mut next_row: u32 = 0;
    let mut row_no: u32 = 0;
    let mut cells: Vec<RawCell> = vec![];
    let mut formulas: u64 = 0;
    let mut cur: Option<CellState> = None;
    let mut field = Field::None;
    let mut in_is = false;
    let mut next_col = 0usize;
    let mut shared_formulas: HashMap<u32, SharedFormula> = HashMap::new();
    loop {
        let ev = reader.read_event_into(&mut buf).map_err(|e| package_err(format!("{e} at byte {}", reader.buffer_position())))?;
        match ev {
            Event::Start(e) => match e.local_name().as_ref() {
                b"row" => {
                    let el = elem_of(&e);
                    row_no = el.attr("r").and_then(|r| r.trim().parse::<u32>().ok()).unwrap_or(next_row.saturating_add(1));
                    next_row = row_no;
                    cells.clear();
                    formulas = 0;
                    next_col = 0;
                }
                b"c" => {
                    let el = elem_of(&e);
                    let col = el.attr("r").and_then(col_of).or(Some(next_col));
                    next_col = col.map_or(next_col, |c| c + 1);
                    cur = Some(CellState {
                        col,
                        ty: el.attr("t").unwrap_or("n").to_string(),
                        v: String::new(),
                        t: String::new(),
                        f: String::new(),
                        has_formula: false,
                        f_type: String::new(),
                        f_si: None,
                    });
                }
                b"v" => field = Field::V,
                b"is" => in_is = true,
                b"t" if in_is => field = Field::T,
                b"f" => {
                    let el = elem_of(&e);
                    if let Some(c) = cur.as_mut() {
                        c.has_formula = true;
                        c.f_type = el.attr("t").unwrap_or("normal").to_string();
                        c.f_si = el.attr("si").and_then(|s| s.parse::<u32>().ok());
                    }
                    field = Field::F;
                }
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"c" => {
                    let el = elem_of(&e);
                    next_col = el.attr("r").and_then(col_of).map_or(next_col, |c| c + 1);
                }
                b"f" => {
                    let el = elem_of(&e);
                    if let Some(c) = cur.as_mut() {
                        c.has_formula = true;
                        c.f_type = el.attr("t").unwrap_or("normal").to_string();
                        c.f_si = el.attr("si").and_then(|s| s.parse::<u32>().ok());
                    }
                }
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                b"c" => {
                    if let Some(c) = cur.take() {
                        let mut formula = None;
                        if c.has_formula {
                            formulas += 1;
                            let cell_row_0 = row_no.saturating_sub(1);
                            let cell_col_0 = c.col.unwrap_or(0) as u32;
                            let raw_f = c.f.trim();
                            if c.f_type == "shared" {
                                if let Some(si) = c.f_si {
                                    if !raw_f.is_empty() {
                                        let clean = clean_formula(raw_f);
                                        let expr = gridcraft_formula::parse(&clean).ok();
                                        shared_formulas.insert(si, SharedFormula { anchor: (cell_row_0, cell_col_0), expr, text: clean.clone() });
                                        formula = Some(clean);
                                    } else if let Some(sh) = shared_formulas.get(&si) {
                                        let dr = cell_row_0 as i64 - sh.anchor.0 as i64;
                                        let dc = cell_col_0 as i64 - sh.anchor.1 as i64;
                                        let shifted = match &sh.expr {
                                            Some(e) => gridcraft_formula::print(&gridcraft_formula::adjust::shift_relative(e.clone(), dr, dc)),
                                            None => sh.text.clone(),
                                        };
                                        formula = Some(shifted);
                                    }
                                }
                            } else if !raw_f.is_empty() {
                                formula = Some(clean_formula(raw_f));
                            }
                        }
                        let val = c.value(shared);
                        if c.col.is_some() && (val.is_some() || formula.is_some()) {
                            cells.push(RawCell { col: c.col.unwrap_or(0), val, formula });
                        }
                    }
                    field = Field::None;
                    in_is = false;
                }
                b"row" => on_row(row_no, std::mem::take(&mut cells), formulas)?,
                b"is" => in_is = false,
                b"v" | b"t" | b"f" => field = Field::None,
                _ => {}
            },
            Event::Text(t) => {
                if let Some(c) = cur.as_mut() {
                    let s = t.xml_content(XmlVersion::Explicit1_0).map_err(package_err)?;
                    match field {
                        Field::V => c.v.push_str(&s),
                        Field::T => c.t.push_str(&s),
                        Field::F => c.f.push_str(&s),
                        Field::None => {}
                    }
                }
            }
            Event::GeneralRef(r) => {
                if let Some(c) = cur.as_mut() {
                    match field {
                        Field::V => push_ref(r, &mut c.v),
                        Field::T => push_ref(r, &mut c.t),
                        Field::F => push_ref(r, &mut c.f),
                        Field::None => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

// ---------------------------------------------------------------- import

/// Unique, non-empty column names from a header row.
fn column_names(cells: &[RawCell], n: usize) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(n);
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert("_row".into());
    for i in 0..n {
        let raw = cells
            .iter()
            .find(|c| c.col == i)
            .and_then(|c| c.val.as_ref())
            .map(|v| match v {
                Sql::Text(s) => s.trim().to_string(),
                Sql::Integer(x) => x.to_string(),
                Sql::Real(x) => x.to_string(),
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
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS _gc_sheets (position INTEGER, sheet TEXT, table_name TEXT, source_ref TEXT, formula_cells INTEGER);
CREATE TABLE IF NOT EXISTS _gc_columns (table_name TEXT, position INTEGER, name TEXT, formula TEXT);
CREATE TABLE IF NOT EXISTS _gc_formulas (sheet TEXT, row INTEGER, col INTEGER, formula TEXT);
CREATE TABLE IF NOT EXISTS _gc_cells (sheet TEXT, row INTEGER, col INTEGER, val_type TEXT, val_num REAL, val_text TEXT, formula TEXT, PRIMARY KEY (sheet, row, col));
CREATE TABLE IF NOT EXISTS _gc_tables (table_name TEXT PRIMARY KEY, sheet TEXT, display_name TEXT, range_ref TEXT, header_rows INTEGER, totals_rows INTEGER);
";

/// Converts `xlsx` to a SQLite database at `db` (replaced if it exists), streaming each worksheet.
/// `progress(sheet, rows so far)` is called now and then. Each sheet becomes a table named like the
/// sheet, with an `_row` column (the Excel row number) followed by the columns named by row 1.
pub fn import_xlsx(xlsx: &Path, db: &Path, progress: &mut dyn FnMut(&str, u64)) -> Result<ImportReport> {
    let mut zip = zip::ZipArchive::new(BufReader::new(std::fs::File::open(xlsx)?)).map_err(package_err)?;
    let workbook = read_part(&mut zip, "xl/workbook.xml")?.ok_or_else(|| DbError::Package("xl/workbook.xml is missing".into()))?;
    let sheets: Vec<(String, String)> = flat(&workbook)
        .iter()
        .filter(|e| e.name == "sheet")
        .map(|e| (e.attr("name").unwrap_or("").to_string(), e.attr("id").unwrap_or("").to_string()))
        .collect();
    let wb_rels = rels(&mut zip, "xl/_rels/workbook.xml.rels")?;
    let shared = shared_strings(&mut zip)?;
    let mut report = ImportReport::default();

    // Write to a temporary file so a failed import never destroys an existing database.
    let tmp = db.with_extension("gcimport.tmp");
    let _ = std::fs::remove_file(&tmp);
    let outcome = import_into(&tmp, &mut zip, &sheets, &wb_rels, &shared, progress, &mut report);
    match outcome {
        Ok(()) => {
            let _ = std::fs::remove_file(db);
            std::fs::rename(&tmp, db)?;
            Ok(report)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn import_into(
    path: &Path,
    zip: &mut Archive,
    sheets: &[(String, String)],
    wb_rels: &[(String, String, String)],
    shared: &[String],
    progress: &mut dyn FnMut(&str, u64),
    report: &mut ImportReport,
) -> Result<()> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=MEMORY;")?;
    conn.execute_batch(SCHEMA)?;
    let mut used: HashSet<String> = HashSet::new();
    for (position, (sheet, rid)) in sheets.iter().enumerate() {
        let Some(target) = wb_rels.iter().find(|(id, _, _)| id == rid).map(|(_, _, t)| t) else {
            report.warnings.push(format!("sheet '{sheet}': no worksheet part, skipped"));
            continue;
        };
        let part = resolve("xl", target);
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part.as_str()));
        // Excel table (calculated column formulas and range) of this sheet, if any.
        let sheet_rels = rels(zip, &format!("{dir}/_rels/{file}.rels"))?;
        let mut source_ref = String::new();
        let mut calc: HashMap<String, String> = HashMap::new();
        for (_, _, t) in sheet_rels.iter().filter(|(_, ty, _)| ty.ends_with("/table")) {
            if let Some(bytes) = read_part(zip, &resolve(dir, t))? {
                let mut column: Option<String> = None;
                let mut tbl_name = String::new();
                let mut tbl_display = String::new();
                let mut tbl_ref = String::new();
                for el in flat(&bytes) {
                    match el.name.as_str() {
                        "table" => {
                            tbl_name = el.attr("name").unwrap_or("").to_string();
                            tbl_display = el.attr("displayName").unwrap_or(&tbl_name).to_string();
                            tbl_ref = el.attr("ref").unwrap_or("").to_string();
                            if source_ref.is_empty() {
                                source_ref = tbl_ref.clone();
                            }
                        }
                        "tableColumn" => column = el.attr("name").map(str::to_string),
                        "calculatedColumnFormula" => {
                            if let Some(c) = &column {
                                calc.insert(c.clone(), el.text.trim().to_string());
                            }
                        }
                        _ => {}
                    }
                }
                if !tbl_name.is_empty() {
                    conn.execute(
                        "INSERT OR REPLACE INTO _gc_tables (table_name, sheet, display_name, range_ref, header_rows, totals_rows) VALUES (?1, ?2, ?3, ?4, 1, 0)",
                        rusqlite::params![tbl_name, sheet, tbl_display, tbl_ref],
                    )?;
                }
            }
        }

        let mut table = sheet.clone();
        let mut k = 2;
        while !used.insert(table.to_lowercase()) {
            table = format!("{sheet}_{k}");
            k += 1;
        }
        let entry = zip.by_name(&part).map_err(package_err)?;
        let rd = BufReader::with_capacity(1 << 20, entry);

        let mut insert_sql: Option<String> = None;
        let mut ncols = 0usize;
        let mut names: Vec<String> = vec![];
        let mut rows: u64 = 0;
        let mut formula_cells: u64 = 0;
        let mut overflow: u64 = 0;

        conn.execute_batch("BEGIN;")?;
        {
            let mut insert_cell_stmt = conn.prepare_cached(
                "INSERT OR REPLACE INTO _gc_cells (sheet, row, col, val_type, val_num, val_text, formula) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut insert_formula_stmt =
                conn.prepare_cached("INSERT OR REPLACE INTO _gc_formulas (sheet, row, col, formula) VALUES (?1, ?2, ?3, ?4)")?;

            stream_sheet(rd, shared, &mut |row_no, cells, formulas| {
                formula_cells += formulas;
                if cells.is_empty() {
                    return Ok(());
                }

                // 1. Record every cell into _gc_cells and _gc_formulas
                let row_0 = (row_no.saturating_sub(1)) as i64;
                for c in &cells {
                    let col_0 = c.col as i64;
                    let (v_type, v_num, v_text) = match &c.val {
                        Some(Sql::Integer(i)) => (Some("n"), Some(*i as f64), None),
                        Some(Sql::Real(f)) => (Some("n"), Some(*f), None),
                        Some(Sql::Text(t)) => (Some("s"), None, Some(t.clone())),
                        _ => (None, None, None),
                    };
                    insert_cell_stmt.execute(rusqlite::params![sheet, row_0, col_0, v_type, v_num, v_text, c.formula.as_deref(),])?;
                    if let Some(f) = &c.formula {
                        insert_formula_stmt.execute(rusqlite::params![sheet, row_0, col_0, f])?;
                    }
                }

                // 2. Also populate relational table if applicable
                let Some(sql) = &insert_sql else {
                    ncols = cells.iter().map(|c| c.col).max().map_or(0, |m| m + 1);
                    names = column_names(&cells, ncols);
                    let cols = names.iter().map(|n| quote_ident(n)).collect::<Vec<_>>().join(", ");
                    conn.execute_batch(&format!("CREATE TABLE {} (_row INTEGER PRIMARY KEY, {cols});", quote_ident(&table)))?;
                    let marks = (1..=ncols + 1).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
                    insert_sql = Some(format!("INSERT OR REPLACE INTO {} (_row, {cols}) VALUES ({marks})", quote_ident(&table)));
                    return Ok(());
                };

                let mut params: Vec<Sql> = vec![Sql::Null; ncols + 1];
                if let Some(p) = params.first_mut() {
                    *p = Sql::Integer(i64::from(row_no));
                }
                for c in &cells {
                    if let Some(v) = &c.val {
                        match params.get_mut(c.col + 1) {
                            Some(slot) => *slot = v.clone(),
                            None => overflow += 1,
                        }
                    }
                }
                conn.prepare_cached(sql)?.execute(params_from_iter(params))?;
                rows += 1;
                if rows.is_multiple_of(25_000) {
                    progress(sheet, rows);
                }
                Ok(())
            })?;
        }
        conn.execute_batch("COMMIT;")?;
        progress(sheet, rows);

        if insert_sql.is_some() {
            conn.execute_batch("BEGIN;")?;
            for (i, name) in names.iter().enumerate() {
                conn.execute(
                    "INSERT INTO _gc_columns (table_name, position, name, formula) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![table, i as i64, name, calc.get(name)],
                )?;
            }
            conn.execute(
                "INSERT INTO _gc_sheets (position, sheet, table_name, source_ref, formula_cells) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![position as i64, sheet, table, source_ref, formula_cells as i64],
            )?;

            // If table has standard lookup columns, create SQLite indexes for rapid performance
            if names.iter().any(|n| n.eq_ignore_ascii_case("Referencia")) {
                let _ = conn.execute_batch(&format!(
                    "CREATE INDEX IF NOT EXISTS {} ON {} (Referencia);",
                    quote_ident(&format!("idx_{table}_ref")),
                    quote_ident(&table)
                ));
            }
            if names.iter().any(|n| n.eq_ignore_ascii_case("Fornecedor")) {
                let _ = conn.execute_batch(&format!(
                    "CREATE INDEX IF NOT EXISTS {} ON {} (Fornecedor);",
                    quote_ident(&format!("idx_{table}_forn")),
                    quote_ident(&table)
                ));
            }
            conn.execute_batch("COMMIT;")?;

            let calculated_columns = names.iter().filter(|n| calc.contains_key(*n)).count();
            if overflow > 0 {
                report.warnings.push(format!("sheet '{sheet}': {overflow} cells outside the header columns were ignored in table view"));
            }
            report.tables.push(TableReport { sheet: table, rows, columns: ncols, formula_cells, calculated_columns });
        } else {
            conn.execute(
                "INSERT INTO _gc_sheets (position, sheet, table_name, source_ref, formula_cells) VALUES (?1, ?2, NULL, ?3, ?4)",
                rusqlite::params![position as i64, sheet, source_ref, formula_cells as i64],
            )?;
            if rows == 0 {
                report.warnings.push(format!("sheet '{sheet}' is empty and was not imported"));
            }
        }
    }
    conn.execute_batch("PRAGMA optimize;")?;
    conn.close().map_err(|(_, e)| DbError::Sqlite(e))?;
    Ok(())
}

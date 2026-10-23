//! Table styles: how a cell inside a table looks (our own palettes, keyed by the familiar
//! `TableStyleLight1…21 / Medium1…28 / Dark1…11` names so files round-trip).

use gridcraft_core::{CellRef, MAX_ROWS, RangeRef};
use gridcraft_model::*;

/// (family, number) from a style name, e.g. `TableStyleMedium2` → ("Medium", 2).
pub fn parse_style_name(name: &str) -> (&'static str, u32) {
    let rest = name.strip_prefix("TableStyle").unwrap_or(name);
    for fam in ["Light", "Medium", "Dark"] {
        if let Some(n) = rest.strip_prefix(fam) {
            let k = n.parse::<u32>().unwrap_or(2).max(1); // the gallery starts at 1; 0 would underflow `n - 1`
            return (fam, k);
        }
    }
    ("Medium", 2)
}

/// Accent theme slot (4..=9) for a style number; 1 and every 7th style are neutral (dk1).
fn accent_for(n: u32) -> Option<u8> {
    let k = (n.saturating_sub(1)) % 7;
    if k == 0 { None } else { Some(3 + k as u8) }
}

/// Header/band/body colours for a table style.
pub struct TableLook {
    pub header_fill: Option<Color>,
    pub header_font: Color,
    pub header_bold: bool,
    pub band_fill: Option<Color>,
    pub body_fill: Option<Color>,
    pub line: Option<Color>,
    pub total_top: Option<Color>,
}

pub fn look(style: &str) -> TableLook {
    let (fam, n) = parse_style_name(style);
    let acc = accent_for(n);
    let base = |tint: i16| match acc {
        Some(a) => Color::Theme(a, tint),
        None => Color::Theme(1, if tint == 0 { 0 } else { tint.clamp(-1000, 1000) }),
    };
    let neutral_light = |tint: i16| match acc {
        Some(a) => Color::Theme(a, tint),
        None => Color::Theme(0, -((1000 - tint).clamp(0, 1000) / 6)),
    };
    match fam {
        "Light" => {
            let group = (n - 1) / 7; // 0: lines only, 1: banded header line, 2: grid
            TableLook {
                header_fill: None,
                header_font: if group == 0 { base(-250) } else { Color::Auto },
                header_bold: true,
                band_fill: Some(neutral_light(800)),
                body_fill: None,
                line: Some(base(0)),
                total_top: Some(base(0)),
            }
        }
        "Dark" => TableLook {
            header_fill: Some(Color::Theme(1, 0)),
            header_font: Color::Theme(0, 0),
            header_bold: true,
            band_fill: Some(base(-250)),
            body_fill: Some(base(0)),
            line: None,
            total_top: Some(Color::Theme(0, 0)),
        },
        _ => TableLook {
            header_fill: Some(base(0)),
            header_font: Color::Theme(0, 0),
            header_bold: true,
            band_fill: Some(neutral_light(800)),
            body_fill: None,
            line: if (n - 1) / 7 == 0 { None } else { Some(base(400)) },
            total_top: Some(base(0)),
        },
    }
}

/// The style a table gives a cell (merged over the cell's own style: explicit fills/fonts win).
pub fn table_cell_style(wb: &Workbook, sheet: usize, t: &Table, c: CellRef) -> Style {
    let own = wb.sheet(sheet).map(|sh| wb.styles.get(sh.style_id(c)).clone()).unwrap_or_default();
    let mut s = own.clone();
    let lk = look(&t.style);
    let is_header = t.header_row && c.row == t.range.start.row;
    let is_total = t.totals_row && c.row == t.range.end.row;
    let data_row = c.row.saturating_sub(t.range.start.row + t.header_row as u32);
    let data_col = c.col - t.range.start.col;
    let explicit_fill = own.fill.pattern != PatternType::None;
    if is_header {
        if let Some(f) = lk.header_fill
            && !explicit_fill
        {
            s.fill = Fill::solid(f);
        }
        if own.font.color == Color::Auto {
            s.font.color = lk.header_font;
        }
        s.font.bold = s.font.bold || lk.header_bold;
        if let Some(l) = lk.line {
            s.borders.bottom = BorderLine { style: BorderStyle::Thin, color: l };
        }
    } else if is_total {
        s.font.bold = true;
        if let Some(l) = lk.total_top {
            s.borders.top = BorderLine { style: BorderStyle::Double, color: l };
        }
    } else {
        let band = (t.banded_rows && data_row.is_multiple_of(2)) || (t.banded_cols && data_col.is_multiple_of(2));
        if !explicit_fill {
            if band {
                if let Some(f) = lk.band_fill {
                    s.fill = Fill::solid(f);
                }
            } else if let Some(f) = lk.body_fill {
                s.fill = Fill::solid(f);
            }
        }
        if let Some(l) = lk.line
            && t.style.contains("Light")
            && c.row == t.range.end.row
        {
            s.borders.bottom = BorderLine { style: BorderStyle::Thin, color: l };
        }
    }
    if (t.first_col && c.col == t.range.start.col) || (t.last_col && c.col == t.range.end.col) {
        s.font.bold = true;
    }
    if t.style.starts_with("TableStyleDark") && !is_header && own.font.color == Color::Auto {
        s.font.color = Color::Theme(0, 0);
    }
    s
}

/// Gallery list of style names.
pub fn gallery() -> Vec<String> {
    let mut v = Vec::new();
    for n in 1..=21 {
        v.push(format!("TableStyleLight{n}"));
    }
    for n in 1..=28 {
        v.push(format!("TableStyleMedium{n}"));
    }
    for n in 1..=11 {
        v.push(format!("TableStyleDark{n}"));
    }
    v
}

/// Editing a table's header row renames the column, as in Excel: an empty or repeated header
/// gets a unique name written back into the cell, and structured references follow the rename.
pub(crate) fn sync_headers(cx: &mut crate::cmd::Ctx) {
    if cx.wb.sheets.iter().all(|s| s.tables.is_empty()) {
        return;
    }
    // (sheet, table name, table range, old column name, new column name)
    let mut renames: Vec<(usize, String, RangeRef, String, String)> = Vec::new();
    let heads: Vec<(usize, RangeRef)> = (0..cx.wb.sheets.len())
        .flat_map(|si| cx.wb.sheet(si).into_iter().flat_map(move |sh| sh.tables.iter().filter(|t| t.header_row).map(move |t| (si, t.range))))
        .map(|(si, r)| (si, RangeRef::new(r.start, CellRef::new(r.start.row, r.end.col))))
        .collect();
    // A structural edit can move anything into a header row: then every header is checked.
    let changed: std::collections::BTreeSet<(usize, CellRef)> = if cx.structural {
        heads.iter().flat_map(|(s, r)| r.iter().map(move |c| (*s, c))).collect()
    } else {
        cx.changed.iter().filter(|(si, c)| heads.iter().any(|(s, r)| s == si && r.contains(*c))).copied().collect()
    };
    // Per table: lower-case column name → column, and the next number to try for a repeated name
    // (both kept up to date, so a wide header row stays linear).
    type Names = (std::collections::HashMap<String, usize>, std::collections::HashMap<String, usize>);
    let mut names: std::collections::HashMap<(usize, usize), Names> = std::collections::HashMap::new();
    for (si, c) in changed {
        let Some(sh) = cx.wb.sheet(si) else { continue };
        let Some(ti) = sh.tables.iter().position(|t| t.header_row && c.row == t.range.start.row && t.range.contains(c)) else { continue };
        let Some(t) = sh.tables.get(ti) else { continue };
        let col = (c.col - t.range.start.col) as usize;
        let Some(old) = t.columns.get(col).map(|x| x.name.clone()) else { continue };
        // A text header names its column as it is: a number format only changes how it looks (an
        // accounting format pads text with spaces), and the file writes the name into the cell.
        let value = sh.value(c);
        let text = match value.as_text() {
            Some(t) => t.to_string(),
            None => crate::display::cell_text(&cx.wb, sh, c),
        };
        // Headers hold constant text: a formula (or a spilled value) there becomes its text.
        let is_text = value.is_text() && sh.cell(c).is_some_and(|x| x.formula.is_none());
        if text == old && is_text {
            continue;
        }
        let (taken, next) = names
            .entry((si, ti))
            .or_insert_with(|| (t.columns.iter().enumerate().map(|(i, x)| (x.name.to_lowercase(), i)).collect(), Default::default()));
        if taken.get(&old.to_lowercase()) == Some(&col) {
            taken.remove(&old.to_lowercase());
        }
        let base = if text.trim().is_empty() { format!("Column{}", col + 1) } else { text.clone() };
        let k = next.entry(base.to_lowercase()).or_insert(2);
        let mut name = base.clone();
        while taken.contains_key(&name.to_lowercase()) {
            name = format!("{base}{k}");
            *k += 1;
        }
        taken.insert(name.to_lowercase(), col);
        let (table, range) = (t.name.clone(), t.range);
        let Some(sh) = cx.wb.sheet_mut(si) else { continue };
        if let Some(x) = sh.tables.get_mut(ti).and_then(|t| t.columns.get_mut(col)) {
            x.name = name.clone();
        }
        // Headers are text: a number typed there, or a generated name, is written back as text.
        if name != text || !is_text {
            let style = sh.style_id(c);
            sh.cells.set(c, Cell { value: gridcraft_core::Value::text(name.as_str()), style, ..Default::default() });
        }
        if !old.eq_ignore_ascii_case(&name) {
            renames.push((si, table, range, old, name));
        }
    }
    if renames.is_empty() {
        return;
    }
    // Structured references follow the new names.
    for i in 0..cx.wb.sheets.len() {
        let formulas: Vec<(CellRef, std::sync::Arc<Formula>)> =
            cx.wb.sheets.get(i).map(|s| s.cells.iter().filter_map(|(c, x)| x.formula.clone().map(|f| (c, f))).collect()).unwrap_or_default();
        for (c, f) in formulas {
            let Some(e) = f.expr() else { continue };
            let mut hit = false;
            let ne = e.map(&mut |x| match x {
                gridcraft_formula::Expr::Struct(mut st) => {
                    for (si, table, range, old, name) in &renames {
                        let this = if st.table.is_empty() { i == *si && range.contains(c) } else { st.table.eq_ignore_ascii_case(table) };
                        if this {
                            for end in [&mut st.col_start, &mut st.col_end].into_iter().flatten() {
                                if end.eq_ignore_ascii_case(old) {
                                    *end = name.clone();
                                    hit = true;
                                }
                            }
                        }
                    }
                    gridcraft_formula::Expr::Struct(st)
                }
                o => o,
            });
            if hit && let Some(cell) = cx.wb.sheet_mut(i).and_then(|s| s.cells.get_mut(c)) {
                cell.formula = Some(std::sync::Arc::new(Formula::from_expr(ne)));
            }
        }
    }
    cx.structural = true;
}

/// Writes the totals row cells (labels and SUBTOTAL formulas).
pub(crate) fn write_totals(sh: &mut gridcraft_model::Sheet, ti: usize) {
    let Some(t) = sh.tables.get(ti).cloned() else { return };
    if !t.totals_row {
        return;
    }
    let row = t.range.end.row;
    for (i, col) in t.columns.iter().enumerate() {
        let c = CellRef::new(row, t.range.start.col + i as u32);
        if let Some(code) = col.totals.subtotal_code() {
            let f = format!("=SUBTOTAL({code},{}[{}])", t.name, col.name);
            sh.set_cell(c, Cell { formula: Some(std::sync::Arc::new(Formula::new(&f))), value: gridcraft_core::Value::Empty, style: sh.style_id(c) });
        } else if let Some(l) = &col.totals_label {
            sh.set_value(c, gridcraft_core::Value::text(l.as_str()));
        } else {
            sh.remove_cell(c);
        }
    }
}

/// When cells are entered or pasted adjacent to or inside an existing table, the table automatically
/// expands to incorporate the new rows and propagates calculated column formulas and cell styles down.
pub(crate) fn auto_expand_tables(cx: &mut crate::cmd::Ctx) {
    if cx.wb.sheets.iter().all(|s| s.tables.is_empty()) {
        return;
    }

    let sheet_count = cx.wb.sheets.len();
    for si in 0..sheet_count {
        let num_tables = cx.wb.sheet(si).map_or(0, |sh| sh.tables.len());
        if num_tables == 0 {
            continue;
        }

        // Changed cells on this sheet
        let sheet_changed: Vec<CellRef> = cx.changed.iter().filter(|(s, _)| *s == si).map(|(_, c)| *c).collect();
        if sheet_changed.is_empty() {
            continue;
        }

        for ti in 0..num_tables {
            let Some(t) = cx.wb.sheet(si).and_then(|sh| sh.tables.get(ti).cloned()) else { continue };
            let c0 = t.range.start.col;
            let c1 = t.range.end.col;
            let header_offset = if t.header_row { 1 } else { 0 };
            let data_start = t.range.start.row + header_offset;
            let data_end = if t.totals_row { t.range.end.row.saturating_sub(1) } else { t.range.end.row };

            // Check if any changed cell on this sheet touches this table's columns
            let touches_table_cols = sheet_changed.iter().any(|c| c.col >= c0 && c.col <= c1);
            if !touches_table_cols {
                continue;
            }

            // Find contiguous non-empty rows extending beyond data_end
            let Some(sh) = cx.wb.sheet(si) else { continue };
            let has_content = |c: CellRef| -> bool { sh.cell(c).is_some_and(|cl| !cl.value.is_empty() || cl.formula.is_some()) };

            let mut curr_end = data_end;
            while curr_end + 1 < MAX_ROWS {
                let next_row = curr_end + 1;

                // Stop if next_row intersects any other table on this sheet
                if sh.tables.iter().enumerate().any(|(other_ti, other)| other_ti != ti && other.range.contains(CellRef::new(next_row, c0))) {
                    break;
                }

                // Check if next_row has non-empty content in the table's columns
                let row_has_content = (c0..=c1).any(|col| has_content(CellRef::new(next_row, col)));
                if !row_has_content {
                    break;
                }

                // Ensure next_row is either part of changed cells or contiguous from an expanded row
                let row_changed = sheet_changed.iter().any(|c| c.row == next_row && c.col >= c0 && c.col <= c1);
                if curr_end > data_end || row_changed {
                    curr_end = next_row;
                } else {
                    break;
                }
            }

            if curr_end <= data_end {
                continue;
            }

            let new_data_end = curr_end;
            let old_data_end = data_end;
            let old_template_row = if old_data_end >= data_start { old_data_end } else { t.range.start.row };

            // Collect template formulas and styles from old_template_row or column metadata
            let mut col_formulas = Vec::new();
            let mut col_styles = Vec::new();

            for (col_idx, table_col) in t.columns.iter().enumerate() {
                let col = c0 + col_idx as u32;
                let template_cell = CellRef::new(old_template_row, col);
                let template_formula = if old_template_row >= data_start { sh.cell(template_cell).and_then(|c| c.formula.clone()) } else { None };
                let col_formula_str = table_col.formula.clone();
                col_formulas.push((template_formula, col_formula_str));
                col_styles.push(sh.style_id(template_cell));
            }

            let old_table_range = t.range;
            let new_end_row = if t.totals_row { new_data_end + 1 } else { new_data_end };

            let Some(shm) = cx.wb.sheet_mut(si) else { continue };

            // Populate formulas & styles in the expanded rows
            let mut new_touched = Vec::new();
            for r in (old_data_end + 1)..=new_data_end {
                for (col_idx, (template_formula, col_formula_str)) in col_formulas.iter().enumerate() {
                    let col = c0 + col_idx as u32;
                    let target_cell = CellRef::new(r, col);
                    let is_empty = shm.cell(target_cell).is_none_or(|c| c.value.is_empty() && c.formula.is_none());
                    let style = col_styles[col_idx];

                    if is_empty {
                        let new_formula: Option<Formula> = if let Some(tf) = template_formula {
                            let dr = (r as i64) - (old_template_row as i64);
                            if let Some(expr) = tf.expr() {
                                let shifted = gridcraft_formula::adjust::shift_relative(expr, dr, 0);
                                Some(Formula::from_expr(shifted))
                            } else {
                                Some(Formula::new(&tf.text))
                            }
                        } else if let Some(f_str) = col_formula_str {
                            let f = Formula::new(f_str);
                            if let Some(expr) = f.expr() {
                                let base_row = data_start as i64;
                                let dr = (r as i64) - base_row;
                                let shifted = gridcraft_formula::adjust::shift_relative(expr, dr, 0);
                                Some(Formula::from_expr(shifted))
                            } else {
                                Some(f)
                            }
                        } else {
                            None
                        };

                        if let Some(formula) = new_formula {
                            shm.set_cell(
                                target_cell,
                                Cell { value: gridcraft_core::Value::Empty, formula: Some(std::sync::Arc::new(formula)), style },
                            );
                            new_touched.push(target_cell);
                        } else if shm.style_id(target_cell) != style {
                            if let Some(cl) = shm.cells.get_mut(target_cell) {
                                cl.style = style;
                            } else {
                                shm.set_cell(target_cell, Cell { value: gridcraft_core::Value::Empty, formula: None, style });
                            }
                        }
                    }
                }
            }

            // Update table range
            if let Some(tbl) = shm.tables.get_mut(ti) {
                tbl.range.end.row = new_end_row;
            }

            // If totals row was on, rewrite it at the new totals row position
            if t.totals_row {
                write_totals(shm, ti);
                for col in c0..=c1 {
                    new_touched.push(CellRef::new(new_end_row, col));
                }
            }

            // Update sheet autofilter range if it was matching the old table range
            if let Some(af) = shm.autofilter.as_mut()
                && af.range == old_table_range
            {
                af.range.end.row = new_end_row;
            }

            for cell in new_touched {
                cx.touch(si, cell);
            }
            cx.structural = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_core::Value;
    use serde_json::json;

    use crate::Session;

    #[test]
    fn editing_a_header_renames_the_column() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["a", "b", "c"], [1, 2, 3]]})).unwrap();
        s.execute("insert.table", json!({"range": "A1:C2", "header": true})).unwrap();
        s.execute("cell.set", json!({"cell": "E1", "input": "=SUM(Table1[a])"})).unwrap();
        s.execute("cell.set", json!({"cell": "A1", "input": "zz"})).unwrap();
        s.execute("edit.clearContents", json!({"range": "B1"})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "ZZ"})).unwrap();
        let get = |s: &mut Session, a: &str| s.execute("cell.get", json!({"cell": a})).unwrap();
        assert_eq!(get(&mut s, "E1")["formula"], "=SUM(Table1[zz])");
        assert_eq!(get(&mut s, "E1")["value"], json!(1.0));
        let d = s.doc().unwrap();
        let sh = d.wb.sheet(0).unwrap();
        let names: Vec<&str> = sh.tables[0].columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["zz", "Column2", "ZZ2"]);
        assert_eq!(sh.value(gridcraft_core::CellRef::new(0, 1)), Value::from("Column2"));
        assert_eq!(sh.value(gridcraft_core::CellRef::new(0, 2)), Value::from("ZZ2"));
        s.execute("range.setValues", json!({"range": "A1", "values": [["x", "x", 5]]})).unwrap();
        let sh = s.doc().unwrap().wb.sheet(0).unwrap();
        let names: Vec<&str> = sh.tables[0].columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["x", "x2", "5"]);
        assert_eq!(sh.value(gridcraft_core::CellRef::new(0, 2)), Value::from("5"));
    }

    #[test]
    fn go_to_a_table_selects_its_data() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["a", "b"], [1, 2], [3, 4]]})).unwrap();
        s.execute("insert.table", json!({"range": "A1:B3", "header": true})).unwrap();
        assert_eq!(s.execute("edit.goTo", json!({"reference": "Table1"})).unwrap()["selection"], "A2:B3");
    }

    #[test]
    fn typing_below_table_auto_expands_and_propagates_formulas() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Item", "Price", "Double"], ["A", 10, "=B2*2"]]})).unwrap();
        s.execute("insert.table", json!({"range": "A1:C2", "header": true})).unwrap();

        let t = &s.doc().unwrap().wb.sheet(0).unwrap().tables[0];
        assert_eq!(t.range.a1(), "A1:C2");

        // Enter "B" into A3 (immediately below table)
        s.execute("cell.set", json!({"cell": "A3", "input": "B"})).unwrap();
        s.execute("cell.set", json!({"cell": "B3", "input": "25"})).unwrap();

        let t = &s.doc().unwrap().wb.sheet(0).unwrap().tables[0];
        assert_eq!(t.range.a1(), "A1:C3");

        // Check that C3 has the propagated formula "=B3*2" and evaluated value 50.0
        let c3 = s.execute("cell.get", json!({"cell": "C3"})).unwrap();
        assert_eq!(c3["formula"], "=B3*2");
        assert_eq!(c3["value"], json!(50.0));
    }

    #[test]
    fn pasting_column_auto_expands_table_and_evaluates_all_rows() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Code", "Qty", "Total"], ["P1", 2, "=B2*10"]]})).unwrap();
        s.execute("insert.table", json!({"range": "A1:C2", "header": true})).unwrap();

        // Paste into A3 (A3..B5)
        s.execute("edit.paste", json!({"at": "A3", "text": "P2\t5\nP3\t7\nP4\t10"})).unwrap();

        let t = &s.doc().unwrap().wb.sheet(0).unwrap().tables[0];
        assert_eq!(t.range.a1(), "A1:C5");

        // Verify formula propagation and calculations
        let get = |s: &mut Session, a: &str| s.execute("cell.get", json!({"cell": a})).unwrap();
        assert_eq!(get(&mut s, "C3")["formula"], "=B3*10");
        assert_eq!(get(&mut s, "C3")["value"], json!(50.0));
        assert_eq!(get(&mut s, "C4")["formula"], "=B4*10");
        assert_eq!(get(&mut s, "C4")["value"], json!(70.0));
        assert_eq!(get(&mut s, "C5")["formula"], "=B5*10");
        assert_eq!(get(&mut s, "C5")["value"], json!(100.0));
    }

    #[test]
    fn edit_not_adjacent_does_not_expand_table() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Item", "Val"], ["A", 1]]})).unwrap();
        s.execute("insert.table", json!({"range": "A1:B2", "header": true})).unwrap();

        // Enter cell at A10 (far below table)
        s.execute("cell.set", json!({"cell": "A10", "input": "Far"})).unwrap();

        let t = &s.doc().unwrap().wb.sheet(0).unwrap().tables[0];
        assert_eq!(t.range.a1(), "A1:B2");
    }

    /// COTACAO-style calculated columns: Linha looks up a price list; Descricao indexes by Linha.
    /// Pasting many Referencia values below the table must expand, autofill formulas, and evaluate.
    #[test]
    fn paste_expands_structured_calc_columns_like_cotacao() {
        let mut s = Session::new();
        s.new_workbook();
        // LISTA_PRECO on columns A:B
        s.execute("range.setValues", json!({
            "range": "A1",
            "values": [
                ["Codigo", "Descricao"],
                ["MB001", "Alpha"],
                ["MB021A", "Bravo"],
                ["MB025A", "Charlie"],
                ["MB019AT", "Delta"]
            ]
        }))
        .unwrap();
        s.execute("insert.table", json!({"range": "A1:B5", "header": true, "name": "LISTA_PRECO"})).unwrap();

        // ORCAMENTO on columns D:F — Referencia | Linha | Descricao
        let linha = concat!(
            "=IF(ORCAMENTO[@Referencia]=\"\",0,",
            "IFERROR(MATCH(ORCAMENTO[@Referencia],LISTA_PRECO[Codigo],0),0))"
        );
        let desc = concat!("=IF(N(ORCAMENTO[@Linha])>0,INDEX(LISTA_PRECO[Descricao],ORCAMENTO[@Linha]),\"\")");
        s.execute("range.setValues", json!({
            "range": "D1",
            "values": [
                ["Referencia", "Linha", "Descricao"],
                ["MB001", linha, desc]
            ]
        }))
        .unwrap();
        s.execute("insert.table", json!({"range": "D1:F2", "header": true, "name": "ORCAMENTO"})).unwrap();

        let get = |s: &mut Session, a: &str| s.execute("cell.get", json!({"cell": a})).unwrap();
        assert_eq!(get(&mut s, "E2")["value"], json!(1.0));
        assert_eq!(get(&mut s, "F2")["value"], json!("Alpha"));

        // Paste several codes below the table (same symptom as the screenshot: D filled, calc cols must follow).
        s.execute("edit.paste", json!({"at": "D3", "text": "MB021A\nMB025A\nMB019AT"})).unwrap();

        let t = &s.doc().unwrap().wb.sheet(0).unwrap().tables.iter().find(|t| t.name == "ORCAMENTO").unwrap();
        assert_eq!(t.range.a1(), "D1:F5", "table must auto-expand over the pasted rows");

        let e3 = get(&mut s, "E3");
        let f3 = get(&mut s, "F3");
        assert!(e3["formula"].as_str().unwrap_or("").contains("ORCAMENTO"), "Linha must autofill, got {e3}");
        assert!(f3["formula"].as_str().unwrap_or("").contains("INDEX"), "Descricao must autofill, got {f3}");
        assert_eq!(e3["value"], json!(2.0), "Linha for MB021A; cell={e3}");
        assert_eq!(f3["value"], json!("Bravo"), "Descricao for MB021A; cell={f3}");
        assert_eq!(get(&mut s, "E4")["value"], json!(3.0));
        assert_eq!(get(&mut s, "F4")["value"], json!("Charlie"));
        assert_eq!(get(&mut s, "E5")["value"], json!(4.0));
        assert_eq!(get(&mut s, "F5")["value"], json!("Delta"));
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;

    #[test]
    fn zero_numbered_styles_do_not_underflow() {
        assert_eq!(parse_style_name("TableStyleLight0"), ("Light", 1));
        assert_eq!(parse_style_name("TableStyleMedium0"), ("Medium", 1));
        for name in ["TableStyleLight0", "TableStyleMedium0", "TableStyleDark0"] {
            let _ = look(name);
        }
    }
}

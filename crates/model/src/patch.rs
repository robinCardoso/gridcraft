//! Compact workbook patches for undo/redo (Fase 5 — docs/plano-responsividade.md).
//!
//! When two workbooks share workbook-level metadata and sheet topology, history can store
//! changed cells and/or per-sheet metadata instead of a full `Arc<Workbook>` snapshot. Formula
//! cached values are ignored when building the cell side so a recalculation cascade does not
//! inflate it.

use std::collections::BTreeMap;
use std::sync::Arc;

use gridcraft_core::CellRef;

use crate::cell::{Cell, CellPicture};
use crate::features::*;
use crate::sheet::{Sheet, SheetViewMode, Visibility};
use crate::style::Color;
use crate::workbook::Workbook;

/// Cells to write on one sheet (`None` removes the stored cell).
#[derive(Clone, Debug, PartialEq)]
pub struct SheetCellPatch {
    pub sheet: usize,
    pub cells: Vec<(CellRef, Option<Cell>)>,
}

/// A set of per-sheet cell writes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellPatch {
    pub sheets: Vec<SheetCellPatch>,
}

impl CellPatch {
    pub fn cell_count(&self) -> usize {
        self.sheets.iter().map(|s| s.cells.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.cell_count() == 0
    }
}

/// Everything on a sheet except the cell store and calc-owned spill maps.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetMeta {
    pub name: String,
    pub cell_pictures: BTreeMap<CellRef, Arc<CellPicture>>,
    pub cols: BTreeMap<u32, LineInfo>,
    pub rows: BTreeMap<u32, LineInfo>,
    pub default_col_width: f32,
    pub default_row_height: f32,
    pub merges: Vec<gridcraft_core::RangeRef>,
    pub freeze: Option<(u32, u32)>,
    pub tab_color: Option<Color>,
    pub visibility: Visibility,
    pub show_gridlines: bool,
    pub show_headings: bool,
    pub show_formulas: bool,
    pub show_zeros: bool,
    pub right_to_left: bool,
    pub zoom: u16,
    pub cond_formats: Vec<CondFormat>,
    pub validations: Vec<Validation>,
    pub tables: Vec<Table>,
    pub autofilter: Option<AutoFilter>,
    pub comments: BTreeMap<CellRef, Comment>,
    pub hyperlinks: BTreeMap<CellRef, Hyperlink>,
    pub charts: Vec<Chart>,
    pub images: Vec<Image>,
    pub shapes: Vec<Shape>,
    pub sparklines: Vec<Sparkline>,
    pub slicers: Vec<Slicer>,
    pub pivots: Vec<PivotTable>,
    pub protection: Option<SheetProtection>,
    pub edit_ranges: Vec<crate::EditRange>,
    pub print: PrintSettings,
    pub view_active: CellRef,
    pub view_top_left: CellRef,
    pub view: SheetViewMode,
    pub preserved: Option<Arc<crate::preserved::Preserved>>,
}

impl SheetMeta {
    fn from_sheet(s: &Sheet) -> Self {
        Self {
            name: s.name.clone(),
            cell_pictures: s.cell_pictures.clone(),
            cols: s.cols.clone(),
            rows: s.rows.clone(),
            default_col_width: s.default_col_width,
            default_row_height: s.default_row_height,
            merges: s.merges.clone(),
            freeze: s.freeze,
            tab_color: s.tab_color,
            visibility: s.visibility,
            show_gridlines: s.show_gridlines,
            show_headings: s.show_headings,
            show_formulas: s.show_formulas,
            show_zeros: s.show_zeros,
            right_to_left: s.right_to_left,
            zoom: s.zoom,
            cond_formats: s.cond_formats.clone(),
            validations: s.validations.clone(),
            tables: s.tables.clone(),
            autofilter: s.autofilter.clone(),
            comments: s.comments.clone(),
            hyperlinks: s.hyperlinks.clone(),
            charts: s.charts.clone(),
            images: s.images.clone(),
            shapes: s.shapes.clone(),
            sparklines: s.sparklines.clone(),
            slicers: s.slicers.clone(),
            pivots: s.pivots.clone(),
            protection: s.protection.clone(),
            edit_ranges: s.edit_ranges.clone(),
            print: s.print.clone(),
            view_active: s.view_active,
            view_top_left: s.view_top_left,
            view: s.view,
            preserved: s.preserved.clone(),
        }
    }

    fn apply_to(&self, s: &mut Sheet) {
        s.name = self.name.clone();
        s.cell_pictures = self.cell_pictures.clone();
        s.cols = self.cols.clone();
        s.rows = self.rows.clone();
        s.default_col_width = self.default_col_width;
        s.default_row_height = self.default_row_height;
        s.merges = self.merges.clone();
        s.freeze = self.freeze;
        s.tab_color = self.tab_color;
        s.visibility = self.visibility;
        s.show_gridlines = self.show_gridlines;
        s.show_headings = self.show_headings;
        s.show_formulas = self.show_formulas;
        s.show_zeros = self.show_zeros;
        s.right_to_left = self.right_to_left;
        s.zoom = self.zoom;
        s.cond_formats = self.cond_formats.clone();
        s.validations = self.validations.clone();
        s.tables = self.tables.clone();
        s.autofilter = self.autofilter.clone();
        s.comments = self.comments.clone();
        s.hyperlinks = self.hyperlinks.clone();
        s.charts = self.charts.clone();
        s.images = self.images.clone();
        s.shapes = self.shapes.clone();
        s.sparklines = self.sparklines.clone();
        s.slicers = self.slicers.clone();
        s.pivots = self.pivots.clone();
        s.protection = self.protection.clone();
        s.edit_ranges = self.edit_ranges.clone();
        s.print = self.print.clone();
        s.view_active = self.view_active;
        s.view_top_left = self.view_top_left;
        s.view = self.view;
        s.preserved = self.preserved.clone();
    }
}

/// Restore sheet metadata (comments, merges, dims, …) without touching the cell store.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetaPatch {
    pub sheet: usize,
    pub meta: SheetMeta,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetaPatch {
    pub sheets: Vec<SheetMetaPatch>,
}

impl MetaPatch {
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty()
    }
}

/// Cell writes plus optional per-sheet metadata restores.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkbookDiff {
    pub cells: CellPatch,
    pub meta: MetaPatch,
}

impl WorkbookDiff {
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.meta.is_empty()
    }

    pub fn cell_count(&self) -> usize {
        self.cells.cell_count()
    }
}

impl Workbook {
    /// Writes every cell in `patch` (copy-on-write per touched sheet/band).
    pub fn apply_cell_patch(&mut self, patch: &CellPatch) {
        for sh in &patch.sheets {
            let Some(sheet) = self.sheet_mut(sh.sheet) else { continue };
            for &(c, ref cell) in &sh.cells {
                match cell {
                    Some(cell) => sheet.cells.set(c, cell.clone()),
                    None => {
                        sheet.cells.remove(c);
                    }
                }
            }
        }
    }

    /// Restores sheet metadata from `patch` without touching cell stores or spill maps.
    pub fn apply_meta_patch(&mut self, patch: &MetaPatch) {
        for sh in &patch.sheets {
            let Some(sheet) = self.sheet_mut(sh.sheet) else { continue };
            sh.meta.apply_to(sheet);
        }
    }

    pub fn apply_workbook_diff(&mut self, diff: &WorkbookDiff) {
        self.apply_meta_patch(&diff.meta);
        self.apply_cell_patch(&diff.cells);
    }

    /// If `after` differs from `self` only in cell-store content and/or sheet metadata (spill maps
    /// ignored — calc owns them), returns `(reverse, forward)` diffs. `None` when workbook-level
    /// metadata or sheet topology differ (caller should keep a full snapshot).
    pub fn history_patches(&self, after: &Workbook) -> Option<(WorkbookDiff, WorkbookDiff)> {
        if !workbook_meta_eq(self, after) {
            return None;
        }
        let mut reverse = WorkbookDiff::default();
        let mut forward = WorkbookDiff::default();
        for (i, (a, b)) in self.sheets.iter().zip(&after.sheets).enumerate() {
            if Arc::ptr_eq(a, b) {
                continue;
            }
            if !sheet_meta_eq(a, b) {
                reverse.meta.sheets.push(SheetMetaPatch { sheet: i, meta: SheetMeta::from_sheet(a) });
                forward.meta.sheets.push(SheetMetaPatch { sheet: i, meta: SheetMeta::from_sheet(b) });
            }
            let refs = a.cells.content_diff(&b.cells);
            if refs.is_empty() {
                continue;
            }
            let mut undo_cells = Vec::with_capacity(refs.len());
            let mut redo_cells = Vec::with_capacity(refs.len());
            for c in refs {
                undo_cells.push((c, a.cells.get(c).cloned()));
                redo_cells.push((c, b.cells.get(c).cloned()));
            }
            reverse.cells.sheets.push(SheetCellPatch { sheet: i, cells: undo_cells });
            forward.cells.sheets.push(SheetCellPatch { sheet: i, cells: redo_cells });
        }
        Some((reverse, forward))
    }

    /// Cell-only patches (returns `None` when sheet metadata also differs). Prefer
    /// [`Self::history_patches`] for undo history.
    pub fn cell_history_patches(&self, after: &Workbook) -> Option<(CellPatch, CellPatch)> {
        let (rev, fwd) = self.history_patches(after)?;
        if !rev.meta.is_empty() {
            return None;
        }
        Some((rev.cells, fwd.cells))
    }
}

fn workbook_meta_eq(a: &Workbook, b: &Workbook) -> bool {
    a.sheets.len() == b.sheets.len()
        && a.active_sheet == b.active_sheet
        && a.names == b.names
        && a.styles == b.styles
        && a.theme == b.theme
        && a.date_system == b.date_system
        && a.calc == b.calc
        && a.props == b.props
        && a.protected_structure == b.protected_structure
        && a.structure_password == b.structure_password
        && a.custom_lists == b.custom_lists
        && a.cell_styles == b.cell_styles
        && a.vba_project == b.vba_project
        && a.csv_delimiter == b.csv_delimiter
        && a.preserved == b.preserved
        && a.source_dir == b.source_dir
}

/// Everything on a sheet except the cell store and calc-owned spill maps.
fn sheet_meta_eq(a: &Sheet, b: &Sheet) -> bool {
    a.name == b.name
        && a.cell_pictures == b.cell_pictures
        && a.cols == b.cols
        && a.rows == b.rows
        && a.default_col_width == b.default_col_width
        && a.default_row_height == b.default_row_height
        && a.merges == b.merges
        && a.freeze == b.freeze
        && a.tab_color == b.tab_color
        && a.visibility == b.visibility
        && a.show_gridlines == b.show_gridlines
        && a.show_headings == b.show_headings
        && a.show_formulas == b.show_formulas
        && a.show_zeros == b.show_zeros
        && a.right_to_left == b.right_to_left
        && a.zoom == b.zoom
        && a.cond_formats == b.cond_formats
        && a.validations == b.validations
        && a.tables == b.tables
        && a.autofilter == b.autofilter
        && a.comments == b.comments
        && a.hyperlinks == b.hyperlinks
        && a.charts == b.charts
        && a.images == b.images
        && a.shapes == b.shapes
        && a.sparklines == b.sparklines
        && a.slicers == b.slicers
        && a.pivots == b.pivots
        && a.protection == b.protection
        && a.edit_ranges == b.edit_ranges
        && a.print == b.print
        && a.view_active == b.view_active
        && a.view_top_left == b.view_top_left
        && a.view == b.view
        && a.preserved == b.preserved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Formula;
    use gridcraft_core::{RangeRef, Value};

    fn set(wb: &mut Workbook, a1: &str, cell: Cell) {
        let c = CellRef::parse(a1).unwrap();
        wb.sheet_mut(0).unwrap().cells.set(c, cell);
    }

    #[test]
    fn cell_patches_round_trip_ignoring_formula_values() {
        let mut before = Workbook::new();
        set(&mut before, "A1", Cell::value(1.0.into()));
        set(&mut before, "B1", Cell::formula(Formula::new("=A1+1")));
        before.sheet_mut(0).unwrap().cells.get_mut(CellRef::parse("B1").unwrap()).unwrap().value = Value::Number(2.0);

        let mut after = before.clone();
        set(&mut after, "A1", Cell::value(10.0.into()));
        // Recalc would refresh B1's cache; simulate it without counting as content.
        after.sheet_mut(0).unwrap().cells.get_mut(CellRef::parse("B1").unwrap()).unwrap().value = Value::Number(11.0);

        let (rev, fwd) = before.history_patches(&after).expect("cells-only");
        assert!(rev.meta.is_empty());
        assert_eq!(rev.cell_count(), 1, "formula value churn must not enter the patch");

        let mut cur = after.clone();
        cur.apply_workbook_diff(&rev);
        assert_eq!(cur.sheet(0).unwrap().cells.get(CellRef::parse("A1").unwrap()).unwrap().value, Value::Number(1.0));
        cur.apply_workbook_diff(&fwd);
        assert_eq!(cur.sheet(0).unwrap().cells.get(CellRef::parse("A1").unwrap()).unwrap().value, Value::Number(10.0));
    }

    #[test]
    fn structural_sheet_change_refuses_a_cell_patch() {
        let before = Workbook::new();
        let mut after = before.clone();
        after.sheets.push(Arc::new(Sheet::new("Sheet2")));
        assert!(before.history_patches(&after).is_none());
        assert!(before.cell_history_patches(&after).is_none());
    }

    #[test]
    fn shared_bands_are_skipped_like_store_diff() {
        let mut before = Workbook::new();
        for r in 0..200u32 {
            set(&mut before, &format!("A{}", r + 1), Cell::value((r as f64).into()));
        }
        let mut after = before.clone();
        set(&mut after, "A6", Cell::value(99.0.into()));
        let (rev, _) = before.history_patches(&after).unwrap();
        assert_eq!(rev.cell_count(), 1);
    }

    #[test]
    fn meta_only_change_builds_a_meta_patch() {
        let before = Workbook::new();
        let mut after = before.clone();
        after.sheet_mut(0).unwrap().merges.push(RangeRef::parse("A1:B2").unwrap());
        after.sheet_mut(0).unwrap().rows.insert(0, LineInfo { size: Some(30.0), custom: true, ..Default::default() });

        let (rev, fwd) = before.history_patches(&after).expect("meta-only");
        assert!(rev.cells.is_empty());
        assert_eq!(rev.meta.sheets.len(), 1);
        assert!(before.cell_history_patches(&after).is_none());

        let mut cur = after.clone();
        // Keep a cell so we can prove meta apply does not wipe the store.
        cur.sheet_mut(0).unwrap().cells.set(CellRef::parse("A1").unwrap(), Cell::value(1.0.into()));
        let cells_ptr = cur.sheet(0).unwrap().cells.clone();
        cur.apply_workbook_diff(&rev);
        assert!(cur.sheet(0).unwrap().merges.is_empty());
        assert!(cur.sheet(0).unwrap().rows.is_empty());
        assert_eq!(cur.sheet(0).unwrap().cells.get(CellRef::parse("A1").unwrap()).unwrap().value, Value::Number(1.0));
        // Cell store Arc must survive a meta-only restore (no accidental CellStore rebuild).
        assert!(cells_ptr.shares_storage_with(&cur.sheet(0).unwrap().cells));
        cur.apply_workbook_diff(&fwd);
        assert_eq!(cur.sheet(0).unwrap().merges.len(), 1);
        assert_eq!(cur.sheet(0).unwrap().rows.get(&0).and_then(|r| r.size), Some(30.0));
    }

    #[test]
    fn cell_and_meta_change_combine_in_one_diff() {
        let mut before = Workbook::new();
        set(&mut before, "A1", Cell::value(1.0.into()));
        let mut after = before.clone();
        set(&mut after, "A1", Cell::value(2.0.into()));
        after.sheet_mut(0).unwrap().comments.insert(
            CellRef::parse("A1").unwrap(),
            Comment { author: "a".into(), text: "note".into(), replies: vec![], threaded: false, resolved: false, visible: true, dates: vec![] },
        );
        let (rev, _) = before.history_patches(&after).unwrap();
        assert_eq!(rev.cell_count(), 1);
        assert_eq!(rev.meta.sheets.len(), 1);
    }
}

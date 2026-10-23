//! Copy-on-write sparse cell storage.
//!
//! Cells live in bands of 64 rows, each band behind an `Arc`. The band map itself sits behind an
//! `Arc` too: cloning a store (Sheet clone / undo snapshot) copies one pointer; editing a cell
//! copies the band map once, then just the touched band.

use std::collections::BTreeMap;
use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef};
use serde::{Deserialize, Serialize};

use crate::cell::Cell;

const BAND: u32 = 64;

/// Whether two cells carry the same user-facing content for undo history: formula text/array and
/// style always count; the cached value counts only when neither side has a formula (constants).
fn content_eq(a: &Cell, b: &Cell) -> bool {
    if a.style != b.style || a.formula != b.formula {
        return false;
    }
    match (&a.formula, &b.formula) {
        (Some(_), Some(_)) => true,
        (None, None) => a.value == b.value,
        _ => false,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Band {
    /// row → (col → cell)
    rows: BTreeMap<u32, BTreeMap<u32, Cell>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Inner {
    bands: BTreeMap<u32, Arc<Band>>,
    len: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CellStore {
    inner: Arc<Inner>,
}

impl PartialEq for CellStore {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner) || *self.inner == *other.inner
    }
}

impl CellStore {
    pub fn new() -> Self {
        Self::default()
    }
    fn inner_mut(&mut self) -> &mut Inner {
        Arc::make_mut(&mut self.inner)
    }
    pub fn len(&self) -> usize {
        self.inner.len
    }
    pub fn is_empty(&self) -> bool {
        self.inner.len == 0
    }
    /// Whether `self` and `other` still share the same band map (true after clone, false after either mutates).
    pub fn shares_storage_with(&self, other: &CellStore) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
    pub fn get(&self, c: CellRef) -> Option<&Cell> {
        self.inner.bands.get(&(c.row / BAND))?.rows.get(&c.row)?.get(&c.col)
    }
    /// Mutable access (copies the store and band if shared).
    pub fn get_mut(&mut self, c: CellRef) -> Option<&mut Cell> {
        let inner = self.inner_mut();
        let band = inner.bands.get_mut(&(c.row / BAND))?;
        if !band.rows.get(&c.row).is_some_and(|r| r.contains_key(&c.col)) {
            return None;
        }
        Arc::make_mut(band).rows.get_mut(&c.row)?.get_mut(&c.col)
    }
    /// Stores a cell; a default (empty, unstyled) cell is removed instead.
    pub fn set(&mut self, c: CellRef, cell: Cell) {
        if cell.is_blank() {
            self.remove(c);
            return;
        }
        let inner = self.inner_mut();
        let band = Arc::make_mut(inner.bands.entry(c.row / BAND).or_default());
        if band.rows.entry(c.row).or_default().insert(c.col, cell).is_none() {
            inner.len += 1;
        }
    }
    pub fn remove(&mut self, c: CellRef) -> Option<Cell> {
        let key = c.row / BAND;
        let inner = self.inner_mut();
        let band = inner.bands.get_mut(&key)?;
        if !band.rows.get(&c.row).is_some_and(|r| r.contains_key(&c.col)) {
            return None;
        }
        let b = Arc::make_mut(band);
        let row = b.rows.get_mut(&c.row)?;
        let old = row.remove(&c.col);
        if row.is_empty() {
            b.rows.remove(&c.row);
        }
        if b.rows.is_empty() {
            inner.bands.remove(&key);
        }
        if old.is_some() {
            inner.len -= 1;
        }
        old
    }
    /// Cells in `r`, row-major. Efficient for sparse data: skips empty bands and rows.
    pub fn iter_range(&self, r: RangeRef) -> impl Iterator<Item = (CellRef, &Cell)> + '_ {
        let (b0, b1) = (r.start.row / BAND, r.end.row / BAND);
        self.inner.bands.range(b0..=b1).flat_map(move |(_, band)| {
            band.rows
                .range(r.start.row..=r.end.row)
                .flat_map(move |(row, cols)| cols.range(r.start.col..=r.end.col).map(move |(col, cell)| (CellRef::new(*row, *col), cell)))
        })
    }
    pub fn iter(&self) -> impl Iterator<Item = (CellRef, &Cell)> + '_ {
        self.inner
            .bands
            .values()
            .flat_map(|band| band.rows.iter().flat_map(|(row, cols)| cols.iter().map(move |(col, cell)| (CellRef::new(*row, *col), cell))))
    }
    /// Cells of one row in `c0..=c1`.
    pub fn row(&self, row: u32, c0: u32, c1: u32) -> impl Iterator<Item = (u32, &Cell)> + '_ {
        self.inner
            .bands
            .get(&(row / BAND))
            .and_then(|b| b.rows.get(&row))
            .into_iter()
            .flat_map(move |cols| cols.range(c0..=c1).map(|(c, cell)| (*c, cell)))
    }
    /// Smallest range containing every stored cell, `None` when empty.
    pub fn used_range(&self) -> Option<RangeRef> {
        let first_band = self.inner.bands.values().next()?;
        let last_band = self.inner.bands.values().next_back()?;
        let r0 = *first_band.rows.keys().next()?;
        let r1 = *last_band.rows.keys().next_back()?;
        let mut c0 = u32::MAX;
        let mut c1 = 0;
        for band in self.inner.bands.values() {
            for cols in band.rows.values() {
                if let (Some(a), Some(b)) = (cols.keys().next(), cols.keys().next_back()) {
                    c0 = c0.min(*a);
                    c1 = c1.max(*b);
                }
            }
        }
        if c0 == u32::MAX {
            return None;
        }
        Some(RangeRef::new(CellRef::new(r0, c0), CellRef::new(r1, c1)))
    }
    /// Last stored row in column `col` at or above `below_or_at` (for Ctrl+arrow navigation).
    pub fn has(&self, c: CellRef) -> bool {
        self.get(c).is_some_and(|cell| !cell.value.is_empty() || cell.formula.is_some())
    }
    /// Positions whose cells differ between `self` and `other` (stored on one side only, or not
    /// equal). Stores that share the same inner `Arc`, and bands both stores share, are skipped.
    pub fn diff(&self, other: &CellStore) -> Vec<CellRef> {
        self.diff_by(other, |a, b| a != b)
    }

    /// Like [`Self::diff`], but formula cells compare by formula and style only — cached values
    /// from recalculation are ignored. Used for undo-by-diff so a cascade of value updates does
    /// not inflate the patch to every dependent cell.
    pub fn content_diff(&self, other: &CellStore) -> Vec<CellRef> {
        self.diff_by(other, |a, b| !content_eq(a, b))
    }

    fn diff_by(&self, other: &CellStore, differ: impl Fn(&Cell, &Cell) -> bool) -> Vec<CellRef> {
        if Arc::ptr_eq(&self.inner, &other.inner) {
            return Vec::new();
        }
        let empty = Band::default();
        let mut keys: Vec<u32> = self.inner.bands.keys().chain(other.inner.bands.keys()).copied().collect();
        keys.sort_unstable();
        keys.dedup();
        let mut out = Vec::new();
        for k in keys {
            let (a, b) = (self.inner.bands.get(&k), other.inner.bands.get(&k));
            if let (Some(a), Some(b)) = (a, b)
                && Arc::ptr_eq(a, b)
            {
                continue;
            }
            let (a, b) = (a.map_or(&empty, |x| x), b.map_or(&empty, |x| x));
            for (row, cols) in &a.rows {
                for (col, cell) in cols {
                    match b.rows.get(row).and_then(|r| r.get(col)) {
                        Some(other) if !differ(cell, other) => {}
                        _ => out.push(CellRef::new(*row, *col)),
                    }
                }
            }
            for (row, cols) in &b.rows {
                for col in cols.keys() {
                    if !a.rows.get(row).is_some_and(|r| r.contains_key(col)) {
                        out.push(CellRef::new(*row, *col));
                    }
                }
            }
        }
        out
    }
    /// Removes every cell in `r` and returns them.
    pub fn take_range(&mut self, r: RangeRef) -> Vec<(CellRef, Cell)> {
        let keys: Vec<CellRef> = self.iter_range(r).map(|(c, _)| c).collect();
        keys.into_iter().filter_map(|c| self.remove(c).map(|cell| (c, cell))).collect()
    }
    /// Rows strictly greater than or equal to `from` moved by `delta` (positive = down). Rows that
    /// would land before `from`... callers delete the vacated block first.
    pub fn shift_rows(&mut self, from: u32, delta: i64) {
        let moved: Vec<(CellRef, Cell)> = {
            let keys: Vec<CellRef> = self.iter().filter(|(c, _)| c.row >= from).map(|(c, _)| c).collect();
            keys.into_iter().filter_map(|c| self.remove(c).map(|cell| (c, cell))).collect()
        };
        for (c, cell) in moved {
            let nr = c.row as i64 + delta;
            if (0..gridcraft_core::MAX_ROWS as i64).contains(&nr) {
                self.set(CellRef::new(nr as u32, c.col), cell);
            }
        }
    }
    /// Moves every cell to the row `f` gives for its current row (cells mapped to `None` or off
    /// the sheet are dropped) in one pass — the batch form of many [`Self::shift_rows`] calls.
    pub fn remap_rows(&mut self, f: impl Fn(u32) -> Option<u32>) {
        let old = std::mem::take(self);
        for (c, cell) in old.iter() {
            if let Some(nr) = f(c.row).filter(|r| *r < gridcraft_core::MAX_ROWS) {
                self.set(CellRef::new(nr, c.col), cell.clone());
            }
        }
    }
    pub fn shift_cols(&mut self, from: u32, delta: i64) {
        let keys: Vec<CellRef> = self.iter().filter(|(c, _)| c.col >= from).map(|(c, _)| c).collect();
        let moved: Vec<(CellRef, Cell)> = keys.into_iter().filter_map(|c| self.remove(c).map(|cell| (c, cell))).collect();
        for (c, cell) in moved {
            let nc = c.col as i64 + delta;
            if (0..gridcraft_core::MAX_COLS as i64).contains(&nc) {
                self.set(CellRef::new(c.row, nc as u32), cell);
            }
        }
    }
    /// Shifts cells within rows `r0..=r1` and columns `>= from` horizontally (Insert/Delete Cells
    /// with shift right/left).
    pub fn shift_cols_in_rows(&mut self, r0: u32, r1: u32, from: u32, delta: i64) {
        let range = RangeRef::new(CellRef::new(r0, from), CellRef::new(r1, gridcraft_core::MAX_COLS - 1));
        let moved = self.take_range(range);
        for (c, cell) in moved {
            let nc = c.col as i64 + delta;
            if (0..gridcraft_core::MAX_COLS as i64).contains(&nc) {
                self.set(CellRef::new(c.row, nc as u32), cell);
            }
        }
    }
    /// Shifts cells within columns `c0..=c1` and rows `>= from` vertically.
    pub fn shift_rows_in_cols(&mut self, c0: u32, c1: u32, from: u32, delta: i64) {
        let range = RangeRef::new(CellRef::new(from, c0), CellRef::new(gridcraft_core::MAX_ROWS - 1, c1));
        let moved = self.take_range(range);
        for (c, cell) in moved {
            let nr = c.row as i64 + delta;
            if (0..gridcraft_core::MAX_ROWS as i64).contains(&nr) {
                self.set(CellRef::new(nr as u32, c.col), cell);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_core::Value;

    fn c(r: u32, col: u32) -> CellRef {
        CellRef::new(r, col)
    }

    #[test]
    fn set_get_remove() {
        let mut s = CellStore::new();
        s.set(c(0, 0), Cell::value(1.0.into()));
        s.set(c(100, 5), Cell::value("x".into()));
        assert_eq!(s.len(), 2);
        assert_eq!(s.get(c(100, 5)).map(|x| x.value.clone()), Some(Value::from("x")));
        assert_eq!(s.used_range().unwrap().a1(), "A1:F101");
        s.set(c(0, 0), Cell::default());
        assert_eq!(s.len(), 1);
        assert!(s.remove(c(100, 5)).is_some());
        assert!(s.is_empty());
        assert!(s.used_range().is_none());
    }

    #[test]
    fn cow_snapshots() {
        let mut s = CellStore::new();
        for r in 0..1000 {
            s.set(c(r, 0), Cell::value((r as f64).into()));
        }
        let snap = s.clone();
        s.set(c(5, 0), Cell::value(99.0.into()));
        assert_eq!(snap.get(c(5, 0)).unwrap().value, Value::Number(5.0));
        assert_eq!(s.get(c(5, 0)).unwrap().value, Value::Number(99.0));
    }

    #[test]
    fn clone_shares_inner_until_edit() {
        let mut s = CellStore::new();
        for r in 0..10_000 {
            s.set(c(r, 0), Cell::value((r as f64).into()));
        }
        let snap = s.clone();
        assert!(std::sync::Arc::ptr_eq(&s.inner, &snap.inner));
        s.set(c(5, 0), Cell::value(99.0.into()));
        assert!(!std::sync::Arc::ptr_eq(&s.inner, &snap.inner));
        assert_eq!(snap.get(c(5, 0)).unwrap().value, Value::Number(5.0));
    }

    #[test]
    fn diff_visits_changed_bands() {
        let mut s = CellStore::new();
        for r in 0..1000 {
            s.set(c(r, 0), Cell::value((r as f64).into()));
        }
        let snap = s.clone();
        assert!(s.diff(&snap).is_empty());
        s.set(c(5, 0), Cell::value(99.0.into()));
        s.set(c(700, 3), Cell::value("new".into()));
        s.remove(c(900, 0));
        assert_eq!(snap.diff(&s), [c(5, 0), c(700, 3), c(900, 0)]);
    }

    #[test]
    fn content_diff_skips_formula_value_churn() {
        let mut s = CellStore::new();
        s.set(c(0, 0), Cell { value: 1.0.into(), formula: Some(std::sync::Arc::new(crate::cell::Formula::new("=1+1"))), style: Default::default() });
        let snap = s.clone();
        s.get_mut(c(0, 0)).unwrap().value = 2.0.into();
        assert!(s.content_diff(&snap).is_empty());
        assert_eq!(s.diff(&snap), [c(0, 0)]);
        s.get_mut(c(0, 0)).unwrap().formula = Some(std::sync::Arc::new(crate::cell::Formula::new("=2+2")));
        assert_eq!(s.content_diff(&snap), [c(0, 0)]);
    }

    #[test]
    fn range_iteration_order() {
        let mut s = CellStore::new();
        s.set(c(70, 2), Cell::value(3.0.into()));
        s.set(c(1, 1), Cell::value(1.0.into()));
        s.set(c(1, 3), Cell::value(2.0.into()));
        s.set(c(500, 0), Cell::value(4.0.into()));
        let v: Vec<f64> = s.iter_range(RangeRef::parse("A1:D100").unwrap()).map(|(_, x)| x.value.as_f64().unwrap()).collect();
        assert_eq!(v, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn shifting() {
        let mut s = CellStore::new();
        s.set(c(2, 0), Cell::value(1.0.into()));
        s.set(c(5, 0), Cell::value(2.0.into()));
        s.shift_rows(3, 2);
        assert!(s.get(c(7, 0)).is_some());
        assert!(s.get(c(2, 0)).is_some());
        s.shift_cols(0, 1);
        assert!(s.get(c(7, 1)).is_some());
    }
}

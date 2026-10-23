//! Dependency graph and recalculation.
//!
//! The graph maps every formula cell to its static precedents (resolved areas). A reverse index
//! answers "which formulas read this cell?": single-cell precedents in a hash map, ranges as row
//! intervals per column (wide ranges per sheet), each an interval index so a lookup costs
//! O(log n + matches) however many ranges share the column. After an edit, the dirty set is the
//! transitive closure of dependents; it is evaluated in topological order, with on-demand
//! evaluation as a safety net for dynamic references. Cycles produce `#CIRC!` unless iterative
//! calculation is on: then a formula read while it is being evaluated gives its previous value,
//! and the formulas that read one are evaluated again until the loop settles
//! (`PassHost::iterate_loops`).

use std::collections::{BTreeSet, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A fast, non-cryptographic hasher for cell keys (FxHash-style multiply-rotate).
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(*b as u64);
        }
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
}

type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
type HashSet<K> = std::collections::HashSet<K, BuildHasherDefault<FxHasher>>;

use gridcraft_core::{Array, CellError, CellRef, RangeRef, Value};
use gridcraft_formula::Expr;
use gridcraft_model::{CalcMode, Workbook};

use crate::eval::{Area, Evaluator, Host, precedents};

pub type Key = (usize, CellRef);

#[derive(Clone, Debug, Default)]
struct Node {
    areas: Vec<Area>,
    dynamic: bool,
}

/// Ranges wider than this many columns go to the per-sheet wide index instead of per-column ones.
const WIDE: u32 = 64;

/// Insertions an [`Intervals`] keeps aside (scanned by every query) before merging them in.
const PENDING: usize = 32;

/// Formula evaluations iterative calculation may spend on one recalculation (passes × formulas
/// evaluated again), however high Maximum Iterations is.
const MAX_ITERATION_WORK: usize = 10_000_000;

/// Row intervals `[r0, r1]`, each with a payload, answering "which intervals overlap rows
/// `a..=b`?" in O(log n + k).
///
/// The intervals are sorted by start with an implicit segment tree of their largest end, so a query
/// only descends into subtrees that hold a match. Removals mark entries dead (re-inserting one
/// revives it); insertions wait in a small sorted set. [`Intervals::settle`] merges them and drops
/// dead entries once either grows, so mutations cost O(log n) amortised.
#[derive(Clone, Debug)]
struct Intervals<T> {
    /// Sorted, without duplicates.
    items: Vec<(u32, u32, T)>,
    dead: Vec<bool>,
    dead_count: usize,
    /// Segment tree over `items`: node `i` has children `2i` and `2i + 1`, leaves start at
    /// `max_end.len() / 2`. Each node holds the largest end below it (dead entries included).
    max_end: Vec<u32>,
    /// Inserted since the last merge.
    pending: BTreeSet<(u32, u32, T)>,
}

impl<T> Default for Intervals<T> {
    fn default() -> Self {
        Intervals { items: Vec::new(), dead: Vec::new(), dead_count: 0, max_end: Vec::new(), pending: BTreeSet::new() }
    }
}

impl<T: Copy + Ord> Intervals<T> {
    fn is_empty(&self) -> bool {
        self.items.len() == self.dead_count && self.pending.is_empty()
    }

    fn insert(&mut self, r0: u32, r1: u32, t: T) {
        let it = (r0, r1, t);
        if let Ok(i) = self.items.binary_search(&it) {
            if let Some(d) = self.dead.get_mut(i)
                && *d
            {
                *d = false;
                self.dead_count -= 1;
            }
            return;
        }
        self.pending.insert(it);
    }

    fn remove(&mut self, r0: u32, r1: u32, t: T) {
        let it = (r0, r1, t);
        if let Ok(i) = self.items.binary_search(&it) {
            if let Some(d) = self.dead.get_mut(i)
                && !*d
            {
                *d = true;
                self.dead_count += 1;
            }
            return;
        }
        self.pending.remove(&it);
    }

    /// Merges pending insertions and drops dead entries when there are enough of them.
    fn settle(&mut self) {
        if self.pending.len() <= PENDING && self.dead_count <= (self.items.len() / 2).max(PENDING) {
            return;
        }
        let mut all: Vec<(u32, u32, T)> = Vec::with_capacity(self.items.len() - self.dead_count + self.pending.len());
        all.extend(self.items.iter().zip(&self.dead).filter(|(_, d)| !**d).map(|(it, _)| *it));
        all.extend(std::mem::take(&mut self.pending));
        // Two sorted runs: the stable sort merges them in linear time.
        all.sort();
        all.dedup();
        let size = all.len().next_power_of_two();
        let mut max_end = vec![0; 2 * size];
        if let Some(leaves) = max_end.get_mut(size..) {
            for (slot, it) in leaves.iter_mut().zip(&all) {
                *slot = it.1;
            }
        }
        for i in (1..size).rev() {
            let m = max_end.get(2 * i).copied().unwrap_or(0).max(max_end.get(2 * i + 1).copied().unwrap_or(0));
            if let Some(slot) = max_end.get_mut(i) {
                *slot = m;
            }
        }
        self.dead = vec![false; all.len()];
        self.dead_count = 0;
        self.items = all;
        self.max_end = max_end;
    }

    /// Calls `f` for every interval overlapping rows `a..=b` (`r0 <= b && r1 >= a`). Returns the
    /// number of tree nodes visited.
    fn query(&self, a: u32, b: u32, f: &mut dyn FnMut(&T)) -> usize {
        for it in &self.pending {
            if it.0 <= b && it.1 >= a {
                f(&it.2);
            }
        }
        // Entries before `p` start at or before row `b`.
        let p = self.items.partition_point(|it| it.0 <= b);
        if p == 0 {
            return 0;
        }
        self.walk(1, 0, self.max_end.len() / 2, p, a, f)
    }

    fn walk(&self, node: usize, lo: usize, hi: usize, p: usize, a: u32, f: &mut dyn FnMut(&T)) -> usize {
        if lo >= p || self.max_end.get(node).is_none_or(|m| *m < a) {
            return 1;
        }
        if hi - lo <= 1 {
            if let (Some(it), Some(false)) = (self.items.get(lo), self.dead.get(lo))
                && it.1 >= a
            {
                f(&it.2);
            }
            return 1;
        }
        let mid = lo + (hi - lo) / 2;
        1 + self.walk(2 * node, lo, mid, p, a, f) + self.walk(2 * node + 1, mid, hi, p, a, f)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    nodes: HashMap<Key, Node>,
    /// single cell → dependents
    cell_deps: HashMap<Key, HashSet<Key>>,
    /// (sheet, col) → dependents covering the whole column (all rows 0..=MAX_ROWS-1)
    whole_col_deps: HashMap<(usize, u32), HashSet<Key>>,
    /// (sheet, col) → row intervals of the ranges (up to `WIDE` columns) that cover the column
    col_deps: HashMap<(usize, u32), Intervals<Key>>,
    /// sheet → row intervals of wider ranges, with their columns: (first, last, dependent)
    wide_deps: HashMap<usize, Intervals<(u32, u32, Key)>>,
    dynamic: HashSet<Key>,
    /// Range indexes changed since the last [`Graph::settle`].
    unsettled_cols: HashSet<(usize, u32)>,
    unsettled_wide: HashSet<usize>,
}

impl Graph {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Index entries of a formula's precedents, without duplicates: single cells, column
    /// intervals and wide ranges.
    fn entries(areas: &[Area]) -> (Vec<Key>, Vec<(usize, u32, u32, u32)>, Vec<(usize, RangeRef)>) {
        let (mut cells, mut cols, mut wide) = (Vec::new(), Vec::new(), Vec::new());
        for a in areas {
            if a.range.is_single() {
                cells.push((a.sheet, a.range.start));
            } else if a.range.width() > WIDE {
                wide.push((a.sheet, a.range));
            } else {
                for c in a.range.start.col..=a.range.end.col {
                    cols.push((a.sheet, c, a.range.start.row, a.range.end.row));
                }
            }
        }
        cells.sort_unstable();
        cells.dedup();
        cols.sort_unstable();
        cols.dedup();
        wide.sort_unstable_by_key(|(s, r)| (*s, r.start, r.end));
        wide.dedup();
        (cells, cols, wide)
    }

    fn insert(&mut self, k: Key, node: Node) {
        self.remove(k);
        let (cells, cols, wide) = Self::entries(&node.areas);
        for c in cells {
            self.cell_deps.entry(c).or_default().insert(k);
        }
        for (s, c, r0, r1) in cols {
            if r0 == 0 && r1 >= gridcraft_core::MAX_ROWS - 1 {
                self.whole_col_deps.entry((s, c)).or_default().insert(k);
            } else {
                self.col_deps.entry((s, c)).or_default().insert(r0, r1, k);
                self.unsettled_cols.insert((s, c));
            }
        }
        for (s, r) in wide {
            self.wide_deps.entry(s).or_default().insert(r.start.row, r.end.row, (r.start.col, r.end.col, k));
            self.unsettled_wide.insert(s);
        }
        if node.dynamic {
            self.dynamic.insert(k);
        }
        self.nodes.insert(k, node);
    }

    fn remove(&mut self, k: Key) {
        let Some(old) = self.nodes.remove(&k) else { return };
        let (cells, cols, wide) = Self::entries(&old.areas);
        for c in cells {
            if let Some(v) = self.cell_deps.get_mut(&c) {
                v.remove(&k);
                if v.is_empty() {
                    self.cell_deps.remove(&c);
                }
            }
        }
        for (s, c, r0, r1) in cols {
            if r0 == 0 && r1 >= gridcraft_core::MAX_ROWS - 1 {
                if let Some(v) = self.whole_col_deps.get_mut(&(s, c)) {
                    v.remove(&k);
                    if v.is_empty() {
                        self.whole_col_deps.remove(&(s, c));
                    }
                }
            } else if let Some(v) = self.col_deps.get_mut(&(s, c)) {
                v.remove(r0, r1, k);
                self.unsettled_cols.insert((s, c));
            }
        }
        for (s, r) in wide {
            if let Some(v) = self.wide_deps.get_mut(&s) {
                v.remove(r.start.row, r.end.row, (r.start.col, r.end.col, k));
                self.unsettled_wide.insert(s);
            }
        }
        self.dynamic.remove(&k);
    }

    /// Tidies the range indexes changed by `insert`/`remove` (queries stay correct without it,
    /// just slower).
    fn settle(&mut self) {
        for key in std::mem::take(&mut self.unsettled_cols) {
            if let Some(v) = self.col_deps.get_mut(&key) {
                v.settle();
                if v.is_empty() {
                    self.col_deps.remove(&key);
                }
            }
        }
        for key in std::mem::take(&mut self.unsettled_wide) {
            if let Some(v) = self.wide_deps.get_mut(&key) {
                v.settle();
                if v.is_empty() {
                    self.wide_deps.remove(&key);
                }
            }
        }
    }

    /// Formulas that read cell `c` of `sheet`.
    pub fn dependents(&self, sheet: usize, c: CellRef, out: &mut Vec<Key>) {
        if let Some(v) = self.cell_deps.get(&(sheet, c)) {
            out.extend(v.iter().copied());
        }
        if let Some(v) = self.whole_col_deps.get(&(sheet, c.col)) {
            out.extend(v.iter().copied());
        }
        if let Some(v) = self.col_deps.get(&(sheet, c.col)) {
            v.query(c.row, c.row, &mut |k| out.push(*k));
        }
        if let Some(v) = self.wide_deps.get(&sheet) {
            v.query(c.row, c.row, &mut |(c0, c1, k)| {
                if *c0 <= c.col && c.col <= *c1 {
                    out.push(*k);
                }
            });
        }
    }

    /// Optimized closure traversal that dedups whole-column dependents across cells in the same column.
    pub fn dependents_closure(&self, sheet: usize, c: CellRef, whole_cols_seen: &mut HashSet<(usize, u32)>, mut emit: impl FnMut(Key)) {
        if let Some(v) = self.cell_deps.get(&(sheet, c)) {
            for &k in v {
                emit(k);
            }
        }
        if whole_cols_seen.insert((sheet, c.col))
            && let Some(v) = self.whole_col_deps.get(&(sheet, c.col))
        {
            for &k in v {
                emit(k);
            }
        }
        if let Some(v) = self.col_deps.get(&(sheet, c.col)) {
            v.query(c.row, c.row, &mut |k| emit(*k));
        }
        if let Some(v) = self.wide_deps.get(&sheet) {
            v.query(c.row, c.row, &mut |(c0, c1, k)| {
                if *c0 <= c.col && c.col <= *c1 {
                    emit(*k);
                }
            });
        }
    }

    /// Formulas that read any cell of `range` (a formula can be listed more than once).
    pub fn dependents_of_range(&self, sheet: usize, range: RangeRef, out: &mut Vec<Key>) {
        if range.count() <= 4096 {
            for c in range.iter() {
                if let Some(v) = self.cell_deps.get(&(sheet, c)) {
                    out.extend(v.iter().copied());
                }
            }
        } else {
            for ((s, c), v) in &self.cell_deps {
                if *s == sheet && range.contains(*c) {
                    out.extend(v.iter().copied());
                }
            }
        }
        let (r0, r1) = (range.start.row, range.end.row);
        let col_count = self.col_deps.len() + self.whole_col_deps.len();
        if (range.width() as usize) <= col_count {
            for col in range.start.col..=range.end.col {
                if let Some(v) = self.whole_col_deps.get(&(sheet, col)) {
                    out.extend(v.iter().copied());
                }
                if let Some(v) = self.col_deps.get(&(sheet, col)) {
                    v.query(r0, r1, &mut |k| out.push(*k));
                }
            }
        } else {
            for ((s, col), v) in &self.whole_col_deps {
                if *s == sheet && range.start.col <= *col && *col <= range.end.col {
                    out.extend(v.iter().copied());
                }
            }
            for ((s, col), v) in &self.col_deps {
                if *s == sheet && range.start.col <= *col && *col <= range.end.col {
                    v.query(r0, r1, &mut |k| out.push(*k));
                }
            }
        }
        if let Some(v) = self.wide_deps.get(&sheet) {
            v.query(r0, r1, &mut |(c0, c1, k)| {
                if *c0 <= range.end.col && range.start.col <= *c1 {
                    out.push(*k);
                }
            });
        }
    }

    /// Precedent areas of a formula cell (for Trace Precedents).
    pub fn precedents_of(&self, k: Key) -> Vec<Area> {
        self.nodes.get(&k).map(|n| n.areas.clone()).unwrap_or_default()
    }
}

/// Cooperative hooks a background calculation shares with its owner (docs/plano-responsividade.md,
/// Fase 3): progress is reported every few hundred formulas, and a cancellation flag the owner
/// (interface thread) can set stops the evaluation as soon as the next check lands. Shared
/// through `Arc`, so the job can hold it while the [`Calc`] runs on its own thread.
#[derive(Default)]
pub struct CalcHooks {
    cancel: AtomicBool,
    /// `(formulas evaluated, total being evaluated)`; a poisoned or replaced callback is skipped.
    progress: Mutex<Option<Box<dyn FnMut(usize, usize) + Send>>>,
}

impl CalcHooks {
    pub fn new() -> Self {
        Self::default()
    }
    /// Requests cancellation: the running calculation stops soon after and reports an incomplete run.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// Sets the progress reporter, called from the calculating thread with `(done, total)`.
    pub fn set_progress(&self, f: Box<dyn FnMut(usize, usize) + Send>) {
        if let Ok(mut slot) = self.progress.lock() {
            *slot = Some(f);
        }
    }
    /// Reports `(work done, total)` to the progress callback, when one is attached. Engine code
    /// outside a recalculation (Solver, Goal Seek) reports through the same hooks.
    pub fn report(&self, done: usize, total: usize) {
        if let Ok(mut slot) = self.progress.lock()
            && let Some(cb) = slot.as_mut()
        {
            cb(done, total);
        }
    }
}

impl std::fmt::Debug for CalcHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalcHooks").field("cancelled", &self.cancelled()).finish_non_exhaustive()
    }
}

/// Calculation state kept beside a workbook.
#[derive(Clone, Debug)]
pub struct Calc {
    pub graph: Graph,
    /// Whether [`Self::graph`] matches the workbook's formulas. Cleared by
    /// [`Self::invalidate_graph`] (structural edits that skip maintenance); restored by
    /// [`Self::rebuild`] / [`Self::update_nodes`]. F9's [`Self::ensure_graph`] rebuilds only when
    /// this is false (docs/plano-responsividade.md, Fase 3 — rebuild incremental).
    graph_valid: bool,
    rng: u64,
    /// Fixed "now" for deterministic tests; `None` = system clock.
    pub fixed_now: Option<f64>,
    /// Cells whose formulas have errors from cycles.
    pub circular: Vec<Key>,
    pub last_recalc_cells: usize,
    /// Background cooperation (progress + cancel); `None` = run to completion, as always.
    pub hooks: Option<Arc<CalcHooks>>,
}

impl Default for Calc {
    fn default() -> Self {
        Calc {
            graph: Graph::default(),
            graph_valid: false,
            rng: 0x9E37_79B9_7F4A_7C15,
            fixed_now: None,
            circular: vec![],
            last_recalc_cells: 0,
            hooks: None,
        }
    }
}

/// The volatile calculation state a background re-run must carry over (docs/plano-responsividade.md,
/// Fase 3): the `rand()` stream position and the fixed "now". The dependency graph is carried on
/// the cloned [`Calc`] when current; otherwise the job rebuilds it. These two seeds must still
/// match the synchronous result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalcSeeds {
    pub rng: u64,
    pub fixed_now: Option<f64>,
}

impl Calc {
    /// The calculation seeds a background re-run must carry over.
    pub fn seeds(&self) -> CalcSeeds {
        CalcSeeds { rng: self.rng, fixed_now: self.fixed_now }
    }
    pub fn set_seeds(&mut self, seeds: CalcSeeds) {
        self.rng = seeds.rng;
        self.fixed_now = seeds.fixed_now;
    }
    /// Attaches background cooperation hooks (progress + cancellation), taking effect from the
    /// next recalculation on.
    pub fn set_hooks(&mut self, hooks: Arc<CalcHooks>) {
        self.hooks = Some(hooks);
    }
}

struct PassHost<'a> {
    wb: &'a Workbook,
    exprs: &'a HashMap<Key, std::sync::Arc<Expr>>,
    results: HashMap<Key, Value>,
    pending: HashSet<Key>,
    in_progress: HashSet<Key>,
    /// New spills found during the pass: anchor → array.
    spills: HashMap<Key, std::sync::Arc<Array>>,
    /// Used range per sheet: the workbook doesn't change during a pass, and working it out walks
    /// every row, so once per sheet rather than once per range read.
    used: HashMap<usize, Option<RangeRef>>,
    rng: &'a mut u64,
    now: f64,
    cycle: bool,
    cycles: Vec<Key>,
    depth: usize,
    /// Iterative calculation: a circular reference reads the cell's previous value instead of
    /// giving `#CIRC!`.
    iterate: bool,
    /// Values from the previous iteration; cells not in it read their stored value.
    prev: HashMap<Key, Value>,
    /// Formulas being evaluated, innermost last (kept only when iterating).
    stack: Vec<Key>,
    /// Formulas found on a circular reference (when iterating).
    members: HashSet<Key>,
    /// Results that read a previous value, directly or through other formulas.
    tainted: HashSet<Key>,
    /// Whether the formula being evaluated has read a previous value.
    taint: bool,
    lookup_col_cache: HashMap<(usize, u32, u32, u32), HashMap<LookupExactKey, u32>>,
    lookup_row_cache: HashMap<(usize, u32, u32, u32), HashMap<LookupExactKey, u32>>,
    val_cache: HashMap<u64, Value>,
    /// Ranges materialized for this pass: many formulas read the same lookup table or criteria
    /// range, and arrays are `Arc`, so a hit is a pointer clone
    /// (docs/plano-responsividade.md, Fase 3 — "cache de range_values por passada").
    ranges: Vec<(usize, RangeRef, Value)>,
    /// Cells held by `ranges` (the memory budget for the cache).
    range_cells: u64,
    /// Equality index per range for this pass: value key → row-major positions. COUNTIFS/SUMIFS
    /// over the same big table build it once instead of scanning per criterion
    /// (docs/plano-responsividade.md, Fase 3).
    criteria: HashMap<(usize, RangeRef), Arc<HashMap<LookupExactKey, Arc<Vec<u32>>>>>,
    /// A cached range would go stale if a spill fills cells during the pass (or the workbook
    /// already has spills): the cache is off for such workbooks, correctness first.
    spills_possible: bool,
    /// Background cooperation: checked inside on-demand `compute` so a deep dependency tree can
    /// stop without waiting for the top-level dirty cell to finish (status bar Cancel).
    hooks: Option<Arc<CalcHooks>>,
    /// Progress denominator for mid-compute reports (`run_scoped`'s total).
    progress_total: usize,
    /// Steps through `compute` (including recursive deps); used to throttle cancel/progress checks.
    compute_steps: usize,
    /// Set when hooks request cancel mid-tree; `run_scoped` aborts the pass without write-back.
    aborted: bool,
}

/// Most cells kept materialized per pass: a bigger range is read normally, so one huge array
/// cannot blow the budget.
const RANGE_CACHE_CELLS: u64 = 1_500_000;
/// Most distinct ranges kept per pass.
const RANGE_CACHE_ENTRIES: usize = 128;

#[derive(Clone, PartialEq, Eq, Hash)]
enum LookupExactKey {
    Text(String),
    Number(u64),
    Bool(bool),
    Empty,
}

fn to_lookup_exact_key(v: &Value) -> Option<LookupExactKey> {
    match v {
        Value::Text(s) => Some(LookupExactKey::Text(s.to_lowercase())),
        // Excel compares numbers at 15 significant digits, so the index keys the rounded value:
        // the map must agree with a linear scan through `lookup_equal`.
        Value::Number(n) => Some(LookupExactKey::Number(gridcraft_core::exact_number_key(*n))),
        Value::Bool(b) => Some(LookupExactKey::Bool(*b)),
        Value::Empty => Some(LookupExactKey::Empty),
        _ => None,
    }
}

impl<'a> PassHost<'a> {
    fn new(
        wb: &'a Workbook,
        exprs: &'a HashMap<Key, std::sync::Arc<Expr>>,
        rng: &'a mut u64,
        now: f64,
        hooks: Option<Arc<CalcHooks>>,
        progress_total: usize,
    ) -> Self {
        PassHost {
            wb,
            exprs,
            results: HashMap::with_capacity_and_hasher(exprs.len(), Default::default()),
            pending: exprs.keys().copied().collect(),
            in_progress: HashSet::default(),
            spills: HashMap::default(),
            used: HashMap::default(),
            rng,
            now,
            cycle: false,
            cycles: vec![],
            depth: 0,
            iterate: false,
            prev: HashMap::default(),
            stack: Vec::new(),
            members: HashSet::default(),
            tainted: HashSet::default(),
            taint: false,
            lookup_col_cache: HashMap::default(),
            lookup_row_cache: HashMap::default(),
            val_cache: HashMap::default(),
            ranges: Vec::new(),
            range_cells: 0,
            criteria: HashMap::default(),
            spills_possible: wb.sheets.iter().any(|s| !s.spill.is_empty() || !s.spill_ranges.is_empty()),
            hooks,
            progress_total: progress_total.max(1),
            compute_steps: 0,
            aborted: false,
        }
    }
}

/// Whether an array result at `k` can't spill: it is in a table, or its area runs off the sheet,
/// overlaps merged cells, holds other cells, or another formula's spill (unless that formula is
/// recalculated in this pass, and so may spill elsewhere now).
fn spill_blocked(host: &PassHost<'_>, (sheet, anchor): Key, a: &Array) -> bool {
    let Some(sh) = host.wb.sheet(sheet) else { return false };
    let end =
        CellRef::new(anchor.row.saturating_add((a.rows as u32).saturating_sub(1)), anchor.col.saturating_add((a.cols as u32).saturating_sub(1)));
    let range = RangeRef::new(anchor, end);
    let own = sh.spill_ranges.get(&anchor);
    let spilled_over = |c: CellRef| {
        !own.is_some_and(|o| o.contains(c))
            && sh.spill_ranges.iter().any(|(o, r)| *o != anchor && r.contains(c) && !host.exprs.contains_key(&(sheet, *o)))
    };
    end.row >= gridcraft_core::MAX_ROWS
        || end.col >= gridcraft_core::MAX_COLS
        || sh.merges.iter().any(|m| m.intersects(&range))
        || sh.tables.iter().any(|t| t.range.contains(anchor))
        || sh.cells.iter_range(range).any(|(c, cell)| c != anchor && (!cell.value.is_empty() || cell.formula.is_some()))
        || sh.spill.range(range.start..=range.end).any(|(c, _)| range.contains(*c) && spilled_over(*c))
}

impl PassHost<'_> {
    fn compute(&mut self, k: Key) -> Value {
        if self.aborted {
            return Value::Empty;
        }
        // On-demand evaluation can recurse through a huge DAG before the top-level dirty loop
        // checks cancel; sample here so Cancel stops a stuck "0 / N" pass.
        self.compute_steps = self.compute_steps.wrapping_add(1);
        if self.compute_steps.is_multiple_of(256)
            && let Some(hooks) = self.hooks.as_ref()
        {
            if hooks.cancelled() {
                self.aborted = true;
                return Value::Empty;
            }
            let done = self.results.len().min(self.progress_total);
            if done > 0 {
                hooks.report(done, self.progress_total);
            }
        }
        if let Some(v) = self.results.get(&k) {
            let v = v.clone();
            if self.iterate && self.tainted.contains(&k) {
                self.taint = true;
            }
            return v;
        }
        let Some(expr) = self.exprs.get(&k) else {
            return self.wb.sheet(k.0).map(|s| s.value(k.1)).unwrap_or_default();
        };
        if self.in_progress.contains(&k) || self.depth > 2000 {
            self.cycle = true;
            if self.iterate {
                return self.circular_read(k);
            }
            self.cycles.push(k);
            return Value::Error(CellError::Circ);
        }
        self.in_progress.insert(k);
        self.depth += 1;
        let outer_taint = std::mem::replace(&mut self.taint, false);
        if self.iterate {
            self.stack.push(k);
        }
        let expr = std::sync::Arc::clone(expr);
        let v = {
            let mut ev = Evaluator::new(self, k.0, k.1);
            ev.value(&expr)
        };
        // A legacy array formula fills the range it was entered in.
        let legacy = self.wb.sheet(k.0).and_then(|s| s.cell(k.1)).and_then(|c| c.formula.as_ref()).and_then(|f| f.array);
        let v = match legacy {
            Some(range) => fit_to_range(v, range),
            None => v,
        };
        self.depth -= 1;
        self.in_progress.remove(&k);
        self.pending.remove(&k);
        if self.iterate {
            self.stack.pop();
            if self.taint {
                self.tainted.insert(k);
            } else {
                self.tainted.remove(&k);
            }
        }
        self.taint |= outer_taint;
        // A blank cell shows as 0 in a formula's result, inside an array as well (`=A1:A3`,
        // FILTER or SORT of a range with blanks). GROUPBY and PIVOTBY lay out blank cells
        // of their own, which stay blank.
        let keep_blanks = matches!(&*expr, Expr::Call(n, _) if matches!(n.as_str(), "GROUPBY" | "PIVOTBY"));
        let v = match v {
            Value::Array(a) => {
                let a = if keep_blanks || !a.data.iter().any(|x| matches!(x, Value::Empty)) {
                    a
                } else {
                    let data = a.data.iter().map(|x| if matches!(x, Value::Empty) { Value::Number(0.0) } else { x.clone() }).collect();
                    Array::new(a.rows, a.cols, data).map(std::sync::Arc::new).unwrap_or(a)
                };
                if a.rows > 1 || a.cols > 1 {
                    self.spills.insert(k, a.clone());
                }
                a.data.first().cloned().unwrap_or_default()
            }
            Value::Empty => Value::Number(0.0),
            v => v,
        };
        // An array that can't spill is #SPILL! for the formulas evaluated after it in this pass too.
        let v = match self.spills.get(&k) {
            Some(a) if spill_blocked(self, k, a) => {
                self.spills.remove(&k);
                Value::Error(CellError::Spill)
            }
            _ => v,
        };
        self.results.insert(k, v.clone());
        v
    }

    /// A formula's previous value for iterative calculation: from the last iteration, else the
    /// value it shows. Blank, and the `#CIRC!` of a loop calculated with iteration off, are 0.
    fn previous(&self, k: Key) -> Value {
        let v = match self.prev.get(&k) {
            Some(v) => v.clone(),
            None => self.wb.sheet(k.0).map(|s| s.value(k.1)).unwrap_or_default(),
        };
        if matches!(v, Value::Empty | Value::Error(CellError::Circ)) { Value::Number(0.0) } else { v }
    }

    /// Reading `k` while it is being evaluated, with iterative calculation on: its previous value.
    /// The formulas around the loop are the ones whose changes decide when iteration stops.
    fn circular_read(&mut self, k: Key) -> Value {
        self.taint = true;
        match self.stack.iter().rposition(|s| *s == k) {
            Some(i) => self.members.extend(self.stack.get(i..).unwrap_or_default().iter().copied()),
            // Too deep to follow: the formula itself is watched.
            None => {
                self.members.insert(k);
            }
        }
        self.previous(k)
    }

    /// Iterative calculation, after a first pass over `order` that met circular references. The
    /// formulas that read a previous value (the loops and what depends on them) are evaluated
    /// again in the same order, each pass starting from the last one's values and reading what it
    /// has already produced (Gauss-Seidel), until no formula on a loop changes by `max_change` or
    /// more, or after `max_iterations` passes in all. Returns the number of passes.
    fn iterate_loops(&mut self, order: &[Key], max_iterations: u32, max_change: f64) -> u32 {
        let again: Vec<Key> = order.iter().copied().filter(|k| self.tainted.contains(k)).collect();
        let set: HashSet<Key> = again.iter().copied().collect();
        // Bounds the work whatever Maximum Iterations says.
        let max_passes = u32::try_from(MAX_ITERATION_WORK / again.len().max(1)).unwrap_or(u32::MAX).max(1);
        let mut passes = 1u32;
        while passes < max_iterations.min(max_passes) && !self.converged(max_change) {
            self.prev = again.iter().filter_map(|k| self.results.remove(k).map(|v| (*k, v))).collect();
            self.pending = set.clone();
            self.spills.retain(|a, _| !set.contains(a));
            for k in &again {
                self.compute(*k);
            }
            passes += 1;
        }
        passes
    }

    /// Whether every formula on a loop changed by less than `max_change` in the last pass (text,
    /// logical and error values: not at all).
    fn converged(&self, max_change: f64) -> bool {
        self.members.iter().all(|k| {
            let Some(new) = self.results.get(k) else { return true };
            match (new, &self.previous(*k)) {
                (Value::Number(a), Value::Number(b)) => {
                    let d = (a - b).abs();
                    d < max_change || d == 0.0
                }
                (new, old) => new == old,
            }
        })
    }
}

/// A legacy (Ctrl+Shift+Enter) array formula's result fitted to the range it was entered in, as
/// Excel does: a single row or column repeats across the range, a single value fills it, cells
/// past the result show `#N/A`, and a result larger than the range is cut off.
fn fit_to_range(v: Value, range: RangeRef) -> Value {
    if range.count() > crate::eval::MAX_CELLS {
        return Value::Error(CellError::Num);
    }
    let (h, w) = (range.height() as usize, range.width() as usize);
    let Value::Array(a) = v else {
        return if h * w == 1 { v } else { Array::new(h, w, vec![v; h * w]).map(Value::from).unwrap_or(Value::Error(CellError::Value)) };
    };
    if h * w == 1 {
        return a.data.first().cloned().unwrap_or_default();
    }
    let mut data = Vec::with_capacity(h * w);
    for r in 0..h {
        for c in 0..w {
            let (r, c) = (if a.rows == 1 { 0 } else { r }, if a.cols == 1 { 0 } else { c });
            data.push(a.get(r, c).cloned().unwrap_or(Value::Error(CellError::NA)));
        }
    }
    Array::new(h, w, data).map(Value::from).unwrap_or(Value::Error(CellError::Value))
}

impl Host for PassHost<'_> {
    fn workbook(&self) -> &Workbook {
        self.wb
    }
    fn spill_range(&mut self, sheet: usize, anchor: CellRef) -> Option<RangeRef> {
        let k = (sheet, anchor);
        if !(self.pending.contains(&k) || self.results.contains_key(&k)) {
            return self.wb.sheet(sheet)?.spill_ranges.get(&anchor).copied();
        }
        // Recalculated in this pass: its new array, unless something blocks it (#SPILL!).
        self.compute(k);
        let arr = self.spills.get(&k)?;
        let end = CellRef::new(
            anchor.row.saturating_add((arr.rows as u32).saturating_sub(1)),
            anchor.col.saturating_add((arr.cols as u32).saturating_sub(1)),
        );
        let range = RangeRef::new(anchor, end);
        let sh = self.wb.sheet(sheet)?;
        let blocked = end.row >= gridcraft_core::MAX_ROWS
            || end.col >= gridcraft_core::MAX_COLS
            || sh.cells.iter_range(range).any(|(c, cell)| c != anchor && (cell.formula.is_some() || !cell.value.is_empty()))
            || sh.merges.iter().any(|m| m.intersects(&range));
        (!blocked).then_some(range)
    }
    fn cell_value(&mut self, sheet: usize, c: CellRef) -> Value {
        let k = (sheet, c);
        if self.pending.contains(&k) || self.results.contains_key(&k) {
            return self.compute(k);
        }
        // A cell covered by a spill computed in this pass.
        for (anchor, arr) in &self.spills {
            if anchor.0 == sheet && c.row >= anchor.1.row && c.col >= anchor.1.col {
                let (dr, dc) = ((c.row - anchor.1.row) as usize, (c.col - anchor.1.col) as usize);
                if dr < arr.rows && dc < arr.cols {
                    if self.iterate && self.tainted.contains(anchor) {
                        self.taint = true;
                    }
                    return arr.get(dr, dc).cloned().unwrap_or_default();
                }
            }
        }
        self.wb.sheet(sheet).map(|s| s.value(c)).unwrap_or_default()
    }
    fn range_array(&mut self, sheet: usize, range: RangeRef) -> Option<Value> {
        if self.spills_possible || !self.spills.is_empty() {
            return None;
        }
        let cells = u64::from(range.height()) * u64::from(range.width());
        if cells == 0 || cells > RANGE_CACHE_CELLS {
            return None;
        }
        if let Some((_, _, v)) = self.ranges.iter().find(|(s, r, _)| *s == sheet && *r == range) {
            return Some(v.clone());
        }
        let data = self.range_values(sheet, range);
        let array = Array::new(range.height() as usize, range.width() as usize, data)?;
        let value = Value::Array(std::sync::Arc::new(array));
        if self.ranges.len() < RANGE_CACHE_ENTRIES && self.range_cells + cells <= RANGE_CACHE_CELLS {
            self.range_cells += cells;
            self.ranges.push((sheet, range, value.clone()));
        }
        Some(value)
    }
    fn equality_positions(&mut self, sheet: usize, range: RangeRef, needle: &Value) -> Option<Arc<Vec<u32>>> {
        // The same guard as the range cache: a spill can fill cells during the pass.
        if self.spills_possible || !self.spills.is_empty() {
            return None;
        }
        let cells = u64::from(range.height()) * u64::from(range.width());
        if cells == 0 || cells > RANGE_CACHE_CELLS {
            return None;
        }
        let key = (sheet, range);
        if !self.criteria.contains_key(&key) {
            let (h, w) = (range.height(), range.width());
            let mut map: HashMap<LookupExactKey, Arc<Vec<u32>>> = HashMap::default();
            for r in 0..h {
                for c in 0..w {
                    let v = self.cell_value(sheet, CellRef::new(range.start.row + r, range.start.col + c));
                    let Some(k) = to_lookup_exact_key(&v) else { continue };
                    let pos = r * w + c;
                    match map.entry(k) {
                        std::collections::hash_map::Entry::Occupied(mut e) => Arc::make_mut(e.get_mut()).push(pos),
                        std::collections::hash_map::Entry::Vacant(e) => {
                            e.insert(Arc::new(vec![pos]));
                        }
                    }
                }
            }
            self.criteria.insert(key, Arc::new(map));
        }
        let needle_key = to_lookup_exact_key(needle)?;
        Some(self.criteria.get(&key)?.get(&needle_key).cloned().unwrap_or_else(|| Arc::new(Vec::new())))
    }
    fn range_values(&mut self, sheet: usize, range: RangeRef) -> Vec<Value> {
        let (h, w) = (range.height() as usize, range.width() as usize);
        let mut out = vec![Value::Empty; h * w];
        let Some(sh) = self.wb.sheet(sheet) else { return out };
        // Sparse walk: only stored cells; formula cells that are (or were) dirty go through
        // `compute`, constants are copied straight from the store.
        let mut formulas: Vec<(usize, CellRef)> = Vec::new();
        for (c, cell) in sh.cells.iter_range(range) {
            let i = (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize;
            if cell.formula.is_some() {
                formulas.push((i, c));
            } else if let Some(slot) = out.get_mut(i) {
                *slot = cell.value.clone();
            }
        }
        for (c, v) in sh.spill.range(range.start..=range.end) {
            if range.contains(*c) {
                let i = (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize;
                if let Some(slot) = out.get_mut(i)
                    && slot.is_empty()
                {
                    *slot = v.clone();
                }
            }
        }
        for (i, c) in formulas {
            let v = self.cell_value(sheet, c);
            if let Some(slot) = out.get_mut(i) {
                *slot = v;
            }
        }
        if !self.spills.is_empty() {
            let new: Vec<(usize, Value)> = self
                .spills
                .iter()
                .filter(|(a, _)| a.0 == sheet)
                .flat_map(|(a, arr)| {
                    let mut v = Vec::new();
                    for r in 0..arr.rows {
                        for cc in 0..arr.cols {
                            let c = CellRef::new(a.1.row.saturating_add(r as u32), a.1.col.saturating_add(cc as u32));
                            if c != a.1 && range.contains(c) {
                                v.push((
                                    (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize,
                                    arr.get(r, cc).cloned().unwrap_or_default(),
                                ));
                            }
                        }
                    }
                    v
                })
                .collect();
            // Iterating: an array spilled by a formula that read a previous value.
            if self.iterate
                && self.spills.iter().any(|(a, arr)| {
                    let end = CellRef::new(
                        a.1.row.saturating_add((arr.rows as u32).saturating_sub(1)),
                        a.1.col.saturating_add((arr.cols as u32).saturating_sub(1)),
                    );
                    a.0 == sheet && self.tainted.contains(a) && RangeRef::new(a.1, end).intersects(&range)
                })
            {
                self.taint = true;
            }
            for (i, v) in new {
                if let Some(slot) = out.get_mut(i) {
                    *slot = v;
                }
            }
        }
        out
    }
    fn visit_range(&mut self, sheet: usize, range: RangeRef, f: &mut dyn FnMut(&Value) -> bool) -> bool {
        let wb = self.wb;
        let Some(sh) = wb.sheet(sheet) else { return false };
        // Formula cells first, in row-major order like `range_values`: reading one calculates it
        // and the pass remembers the value, so the walk below gets the same value again without
        // this method holding any.
        for (c, cell) in sh.cells.iter_range(range) {
            if cell.formula.is_some() {
                let _ = self.cell_value(sheet, c);
            }
        }
        // Spilled values sit between the stored cells (and the formulas just calculated may have
        // produced some): leave such blocks to `range_values`.
        if self.spills.keys().any(|a| a.0 == sheet) || sh.spill.range(range.start..=range.end).any(|(c, _)| range.contains(*c)) {
            return false;
        }
        for (c, cell) in sh.cells.iter_range(range) {
            let go = if cell.formula.is_some() { f(&self.cell_value(sheet, c)) } else { f(&cell.value) };
            if !go {
                break;
            }
        }
        true
    }
    fn used_range(&mut self, sheet: usize) -> Option<RangeRef> {
        let wb = self.wb;
        *self.used.entry(sheet).or_insert_with(|| wb.sheet(sheet).and_then(|s| s.used_range()))
    }
    fn now_serial(&self) -> f64 {
        self.now
    }
    fn random(&mut self) -> f64 {
        // xorshift64*
        let mut x = *self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn lookup_find_row(&mut self, sheet: usize, search_col: u32, r0: u32, r1: u32, needle: &Value) -> Option<u32> {
        let wild = matches!(needle, Value::Text(t) if gridcraft_functions::has_wildcards(t));
        if wild {
            return (r0..=r1).find(|&r| {
                let key = self.cell_value(sheet, CellRef::new(r, search_col));
                gridcraft_functions::lookup_equal(needle, &key, true)
            });
        }

        let Some(needle_key) = to_lookup_exact_key(needle) else {
            return (r0..=r1).find(|&r| {
                let key = self.cell_value(sheet, CellRef::new(r, search_col));
                gridcraft_functions::lookup_equal(needle, &key, false)
            });
        };

        let key = (sheet, search_col, r0, r1);
        if !self.lookup_col_cache.contains_key(&key) {
            let mut map = HashMap::default();
            for r in r0..=r1 {
                let val = self.cell_value(sheet, CellRef::new(r, search_col));
                if let Some(k) = to_lookup_exact_key(&val) {
                    map.entry(k).or_insert(r);
                }
            }
            self.lookup_col_cache.insert(key, map);
        }
        self.lookup_col_cache.get(&key).and_then(|map| map.get(&needle_key).copied())
    }
    fn lookup_find_col(&mut self, sheet: usize, search_row: u32, c0: u32, c1: u32, needle: &Value) -> Option<u32> {
        let wild = matches!(needle, Value::Text(t) if gridcraft_functions::has_wildcards(t));
        if wild {
            return (c0..=c1).find(|&c| {
                let key = self.cell_value(sheet, CellRef::new(search_row, c));
                gridcraft_functions::lookup_equal(needle, &key, true)
            });
        }

        let Some(needle_key) = to_lookup_exact_key(needle) else {
            return (c0..=c1).find(|&c| {
                let key = self.cell_value(sheet, CellRef::new(search_row, c));
                gridcraft_functions::lookup_equal(needle, &key, false)
            });
        };

        let key = (sheet, search_row, c0, c1);
        if !self.lookup_row_cache.contains_key(&key) {
            let mut map = HashMap::default();
            for c in c0..=c1 {
                let val = self.cell_value(sheet, CellRef::new(search_row, c));
                if let Some(k) = to_lookup_exact_key(&val) {
                    map.entry(k).or_insert(c);
                }
            }
            self.lookup_row_cache.insert(key, map);
        }
        self.lookup_row_cache.get(&key).and_then(|map| map.get(&needle_key).copied())
    }
    fn get_cached_val(&mut self, key: u64) -> Option<Value> {
        self.val_cache.get(&key).cloned()
    }
    fn set_cached_val(&mut self, key: u64, val: Value) {
        self.val_cache.insert(key, val);
    }
}

impl Calc {
    pub fn new() -> Self {
        Self::default()
    }

    fn now(&self) -> f64 {
        if let Some(n) = self.fixed_now {
            return n;
        }
        now_serial()
    }

    /// Whether the dependency graph is known to match the workbook's formulas (safe to skip a
    /// full rebuild on F9).
    pub fn graph_is_current(&self) -> bool {
        self.graph_valid
    }

    /// Marks the graph unusable until the next [`Self::rebuild`] (structural edits that defer
    /// recalculation: row/col insert/delete, sheet rename, etc.).
    pub fn invalidate_graph(&mut self) {
        self.graph_valid = false;
    }

    /// Rebuilds the graph from scratch and marks it current.
    /// Returns `false` if a background caller cancelled mid-rebuild (`Calc::hooks`): the graph is
    /// left invalid so a later F9 rebuilds again instead of trusting a partial index.
    pub fn rebuild(&mut self, wb: &Workbook) -> bool {
        self.graph = Graph::default();
        self.graph_valid = false;
        // Count formulas first so progress has a real denominator (and cancel can stop before the
        // heavy precedents walk on a huge workbook).
        let mut total = 0usize;
        for sheet in &wb.sheets {
            for (_, cell) in sheet.cells.iter() {
                if cell.formula.as_ref().is_some_and(|f| f.expr().is_some()) {
                    total += 1;
                }
            }
        }
        let total = total.max(1);
        if let Some(hooks) = self.hooks.as_ref() {
            if hooks.cancelled() {
                return false;
            }
            hooks.report(0, total);
        }
        let mut built = 0usize;
        for (si, sheet) in wb.sheets.iter().enumerate() {
            for (c, cell) in sheet.cells.iter() {
                if let Some(f) = &cell.formula
                    && let Some(e) = f.expr()
                {
                    let (areas, dynamic) = precedents(wb, si, &e);
                    self.graph.insert((si, c), Node { areas, dynamic });
                    built += 1;
                    if built == 1 || built.is_multiple_of(2048) {
                        if let Some(hooks) = self.hooks.as_ref() {
                            if hooks.cancelled() {
                                self.graph = Graph::default();
                                self.graph_valid = false;
                                return false;
                            }
                            hooks.report(built.min(total), total);
                        }
                    }
                }
            }
        }
        if let Some(hooks) = self.hooks.as_ref()
            && hooks.cancelled()
        {
            self.graph = Graph::default();
            self.graph_valid = false;
            return false;
        }
        self.graph.settle();
        self.graph_valid = true;
        if let Some(hooks) = self.hooks.as_ref() {
            hooks.report(built.min(total), total);
        }
        true
    }

    /// Ensures the graph matches `wb`: no-op when [`Self::graph_is_current`], otherwise a full
    /// [`Self::rebuild`]. Used by F9 so a workbook whose graph was kept up to date by
    /// [`Self::cells_changed`] / [`Self::update_nodes`] skips the O(formulas) rebuild.
    /// Returns `false` only when a background rebuild was cancelled.
    pub fn ensure_graph(&mut self, wb: &Workbook) -> bool {
        if self.graph_valid {
            return true;
        }
        // A full rebuild can take seconds on big workbooks; nudge progress so a background job
        // is not stuck at 0 % with no reports until `run_scoped` starts.
        if let Some(hooks) = self.hooks.as_ref() {
            if hooks.cancelled() {
                return false;
            }
            hooks.report(0, 1);
        }
        self.rebuild(wb)
    }

    /// Updates graph nodes for `changed` cells only (insert/remove precedents), without evaluating.
    /// Keeps the graph current when evaluation is deferred to a background F9 job. No-op when the
    /// graph was [`Self::invalidate_graph`]'d — only a full rebuild is safe then.
    pub fn update_nodes(&mut self, wb: &mut Workbook, changed: &[Key]) {
        if !self.graph_valid {
            return;
        }
        let _ = self.apply_node_updates(wb, changed);
    }

    /// Inserts/removes graph nodes for `changed` and applies spill/#NAME? side effects. Returns the
    /// spill ranges cleared when a formula became a constant (seeds for dependents).
    fn apply_node_updates(&mut self, wb: &mut Workbook, changed: &[Key]) -> Vec<(usize, RangeRef)> {
        let mut cleared: Vec<(usize, RangeRef)> = Vec::new();
        for &k in changed {
            let formula = wb.sheet(k.0).and_then(|s| s.cell(k.1)).and_then(|c| c.formula.clone());
            match formula.and_then(|f| f.expr()) {
                Some(e) => {
                    let (areas, dynamic) = precedents(wb, k.0, &e);
                    self.graph.insert(k, Node { areas, dynamic });
                }
                None => {
                    self.graph.remove(k);
                    // A formula replaced by a constant (or cleared) takes its spilled values with it.
                    if let Some(sh) = wb.sheet_mut(k.0)
                        && let Some(r) = sh.spill_ranges.remove(&k.1)
                    {
                        for c in r.iter() {
                            sh.spill.remove(&c);
                        }
                        cleared.push((k.0, r));
                    }
                    // A formula that doesn't parse evaluates to #NAME?.
                    if let Some(cell) = wb.sheet_mut(k.0).and_then(|s| s.cells.get_mut(k.1))
                        && cell.formula.is_some()
                    {
                        cell.value = Value::Error(CellError::Name);
                    }
                }
            }
        }
        self.graph.settle();
        self.graph_valid = true;
        cleared
    }

    /// Recalculates every formula (F9 / Ctrl+Alt+F9, and after loading).
    /// Returns `false` only when a background caller cancelled (`Calc::hooks`).
    pub fn recalc_all(&mut self, wb: &mut Workbook) -> bool {
        if !self.ensure_graph(wb) {
            return false;
        }
        // Spills are laid out again from the formulas where they are now (a structural edit moves
        // or deletes the formulas, not the values they spilled).
        for i in 0..wb.sheets.len() {
            if wb.sheet(i).is_some_and(|s| !s.spill_ranges.is_empty() || !s.spill.is_empty())
                && let Some(sh) = wb.sheet_mut(i)
            {
                sh.spill.clear();
                sh.spill_ranges.clear();
            }
        }
        let all: Vec<Key> = self.graph.nodes.keys().copied().collect();
        self.run(wb, all, true)
    }

    /// Recalculates after loading: builds the dependency graph and only evaluates formulas
    /// that do not already have a computed value or require dynamic-array spilling.
    /// Formulas whose values were pre-calculated and saved by Excel are preserved instantly.
    pub fn recalc_on_load(&mut self, wb: &mut Workbook) {
        let _ = self.rebuild(wb);
        for i in 0..wb.sheets.len() {
            if wb.sheet(i).is_some_and(|s| !s.spill_ranges.is_empty() || !s.spill.is_empty())
                && let Some(sh) = wb.sheet_mut(i)
            {
                sh.spill.clear();
                sh.spill_ranges.clear();
            }
        }
        let mut needed: Vec<Key> = Vec::new();
        for &k in self.graph.nodes.keys() {
            if let Some(cell) = wb.sheet(k.0).and_then(|s| s.cell(k.1))
                && (cell.value.is_empty() || cell.formula.as_ref().is_some_and(|f| f.array.is_some()))
            {
                needed.push(k);
            }
        }
        if !needed.is_empty() {
            if needed.len() > 1_000 {
                let active = wb.active_sheet;
                needed.retain(|k| {
                    k.0 == active || wb.sheet(k.0).and_then(|s| s.cell(k.1)).is_some_and(|c| c.formula.as_ref().is_some_and(|f| f.array.is_some()))
                });
                needed.sort_by_key(|k| (k.1.row, k.1.col));
                needed.truncate(1_000);
                if !needed.is_empty() {
                    self.run_scoped(wb, needed, false, Some(active));
                }
            } else {
                self.run(wb, needed, false);
            }
        }
    }

    /// Recalculates uncomputed formulas on the given sheet, up to `limit` formulas.
    /// If `limit == 0`, all uncomputed formulas on the sheet are recalculated.
    pub fn recalc_sheet_scoped(&mut self, wb: &mut Workbook, sheet: usize, limit: usize) {
        let mut needed: Vec<Key> = Vec::new();
        for &k in self.graph.nodes.keys() {
            if k.0 == sheet
                && let Some(cell) = wb.sheet(k.0).and_then(|s| s.cell(k.1))
                && (cell.value.is_empty() || cell.formula.as_ref().is_some_and(|f| f.array.is_some()))
            {
                needed.push(k);
            }
        }
        if !needed.is_empty() {
            needed.sort_by_key(|k| (k.1.row, k.1.col));
            if limit > 0 && needed.len() > limit {
                needed.truncate(limit);
            }
            self.run_scoped(wb, needed, false, Some(sheet));
        }
    }

    /// Recalculates all uncomputed formulas on the given sheet.
    pub fn recalc_sheet(&mut self, wb: &mut Workbook, sheet: usize) {
        self.recalc_sheet_scoped(wb, sheet, 0);
    }

    /// Updates the graph for cells whose content changed and recalculates what depends on them.
    /// Returns `false` only when a background caller cancelled (`Calc::hooks`).
    pub fn cells_changed(&mut self, wb: &mut Workbook, changed: &[Key]) -> bool {
        // A structurally invalidated graph must be rebuilt before patching nodes; otherwise
        // keys/areas from before the edit would mix with the new ones.
        if !self.graph_valid && !self.graph.is_empty() && !self.rebuild(wb) {
            return false;
        }
        let cleared = self.apply_node_updates(wb, changed);
        if wb.calc.mode == CalcMode::Manual {
            // Only the edited formulas themselves are evaluated.
            let own: Vec<Key> = changed.iter().copied().filter(|k| self.graph.nodes.contains_key(k)).collect();
            return self.run(wb, own, false);
        }
        let mut seeds: Vec<Key> = changed.to_vec();
        let mut emptied: Vec<Key> = Vec::new();
        // Typing into (or clearing) a cell a dynamic array spills over (or would spill over)
        // re-evaluates the anchor, which may now be blocked or unblocked.
        for &k in changed {
            if let Some(sh) = wb.sheet(k.0) {
                for (anchor, r) in &sh.spill_ranges {
                    if *anchor != k.1 && r.contains(k.1) {
                        seeds.push((k.0, *anchor));
                    }
                }
                for (c, cell) in sh.cells.iter_range(RangeRef::new(CellRef::new(k.1.row.saturating_sub(64), k.1.col.saturating_sub(64)), k.1)) {
                    if c != k.1 && cell.value == Value::Error(CellError::Spill) {
                        seeds.push((k.0, c));
                    }
                }
                if !sh.cells.has(k.1) {
                    match emptied.iter_mut().find(|x| x.0 == k.0) {
                        Some(x) => x.1 = CellRef::new(x.1.row.max(k.1.row), x.1.col.max(k.1.col)),
                        None => emptied.push(k),
                    }
                }
            }
        }
        // An emptied cell can unblock an anchor any distance above or to the left of it.
        for (si, end) in emptied {
            for &(s, c) in self.graph.nodes.keys() {
                if s == si && c.row <= end.row && c.col <= end.col && wb.sheet(s).is_some_and(|sh| sh.value(c) == Value::Error(CellError::Spill)) {
                    seeds.push((s, c));
                }
            }
        }
        // Spill areas of changed anchors also change.
        for &k in changed {
            if let Some(r) = wb.sheet(k.0).and_then(|s| s.spill_ranges.get(&k.1)) {
                seeds.extend(r.iter().map(|c| (k.0, c)));
            }
        }
        for (si, r) in &cleared {
            seeds.extend(r.iter().map(|c| (*si, c)));
        }
        if let Some(hooks) = self.hooks.as_ref()
            && hooks.cancelled()
        {
            return false;
        }
        let t0 = prof_now();
        let dirty = self.dirty_closure(wb, &seeds);
        let t1 = prof_now();
        let n = dirty.len();
        // Automatic must cascade the full dirty set — including other sheets. Truncating to the
        // active sheet when n > 1_000 left cross-sheet dependents stale and made formulas on the
        // active sheet that read those sheets look "static" until a tab switch's scoped refill.
        // Large cascades are deferred by the engine (`defer_structural` + `CASCADE_JOB_CELLS` →
        // `recalc_pending` / F9 job), not discarded here. Load may still scope (`recalc_on_load`).
        let ok = self.run(wb, dirty, false);
        if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
            eprintln!("recalc: {} seeds, closure {} cells {:.1} ms, run {:.1} ms", seeds.len(), n, t1 - t0, prof_now() - t1);
        }
        ok
    }

    /// Whether the recalculation cascade from `seeds` would evaluate more than `limit` formulas.
    /// Counts with an early exit, so a small edit costs a bounded walk while a huge one is
    /// detected fast (docs/plano-responsividade.md, Fase 3: cascades past the limit go to a
    /// background job instead of blocking the interface thread).
    pub fn cascade_exceeds(&self, seeds: &[Key], limit: usize) -> bool {
        if limit == 0 {
            return true;
        }
        let mut seen: HashSet<Key> = HashSet::default();
        let mut queue: VecDeque<Key> = VecDeque::new();
        let mut whole_cols_seen: HashSet<(usize, u32)> = HashSet::default();
        let mut count = 0usize;
        let push = |k: Key, queue: &mut VecDeque<Key>, seen: &mut HashSet<Key>, count: &mut usize| {
            if seen.insert(k) {
                *count += 1;
                queue.push_back(k);
            }
        };
        for &k in seeds {
            if self.graph.nodes.contains_key(&k) {
                push(k, &mut queue, &mut seen, &mut count);
            }
            self.graph.dependents_closure(k.0, k.1, &mut whole_cols_seen, |d| push(d, &mut queue, &mut seen, &mut count));
        }
        for &k in &self.graph.dynamic {
            push(k, &mut queue, &mut seen, &mut count);
        }
        while let Some(k) = queue.pop_front() {
            if count > limit {
                return true;
            }
            self.graph.dependents_closure(k.0, k.1, &mut whole_cols_seen, |d| push(d, &mut queue, &mut seen, &mut count));
        }
        count > limit
    }

    fn dirty_closure(&self, _wb: &Workbook, seeds: &[Key]) -> Vec<Key> {
        let mut seen: HashSet<Key> = HashSet::default();
        let mut queue: VecDeque<Key> = VecDeque::new();
        let mut whole_cols_seen: HashSet<(usize, u32)> = HashSet::default();

        let push = |k: Key, queue: &mut VecDeque<Key>, seen: &mut HashSet<Key>| {
            if seen.insert(k) {
                queue.push_back(k);
            }
        };

        for &k in seeds {
            if self.graph.nodes.contains_key(&k) {
                push(k, &mut queue, &mut seen);
            }
            self.graph.dependents_closure(k.0, k.1, &mut whole_cols_seen, |d| {
                push(d, &mut queue, &mut seen);
            });
        }
        for &k in &self.graph.dynamic {
            push(k, &mut queue, &mut seen);
        }
        while let Some(k) = queue.pop_front() {
            self.graph.dependents_closure(k.0, k.1, &mut whole_cols_seen, |d| {
                push(d, &mut queue, &mut seen);
            });
        }
        seen.into_iter().collect()
    }

    /// Evaluates the dirty set and writes results (and spills) back.
    fn run(&mut self, wb: &mut Workbook, dirty: Vec<Key>, full: bool) -> bool {
        self.run_scoped(wb, dirty, full, None)
    }

    /// Evaluates the dirty set, optionally scoped to a single sheet to avoid cascading recalculation across massive workbooks on load.
    /// Returns `false` only when a background caller cancelled the calculation (`Calc::hooks`);
    /// the workbook is then left as it was for the evaluated subset.
    fn run_scoped(&mut self, wb: &mut Workbook, mut dirty: Vec<Key>, full: bool, scope_sheet: Option<usize>) -> bool {
        let mut dynamic_again = !self.graph.dynamic.is_empty();
        // Progress denominator: every formula in the graph (a full recalc evaluates all of it;
        // an incremental one evaluates no more than its dirty set and what spills re-open).
        let total = self.graph.len().max(dirty.len()).max(1);
        let mut done = 0usize;
        for _round in 0..8 {
            if dirty.is_empty() {
                break;
            }
            if let Some(hooks) = self.hooks.as_ref() {
                if hooks.cancelled() {
                    return false;
                }
                // Report before prep (sort / expr map / PassHost) so Cancel is meaningful while a
                // 640k dirty set is still being prepared — and the bar shows the real N early.
                if done == 0 {
                    hooks.report(0, total);
                }
            }
            // Row-major order is a good topological guess; on-demand evaluation fixes the rest.
            // Active/scoped sheet cells are evaluated first so visible cells update earlier.
            let target_sheet = scope_sheet.or(Some(wb.active_sheet));
            dirty.sort_by_key(|(s, c)| (target_sheet.is_some_and(|ts| *s != ts), *s, c.row, c.col));
            if let Some(hooks) = self.hooks.as_ref()
                && hooks.cancelled()
            {
                return false;
            }
            let mut exprs: HashMap<Key, std::sync::Arc<Expr>> = HashMap::with_capacity_and_hasher(dirty.len(), Default::default());
            for (i, k) in dirty.iter().enumerate() {
                if i.is_multiple_of(8192)
                    && let Some(hooks) = self.hooks.as_ref()
                    && hooks.cancelled()
                {
                    return false;
                }
                if let Some(e) = wb.sheet(k.0).and_then(|s| s.cell(k.1)).and_then(|c| c.formula.as_ref()).and_then(|f| f.expr_arc()) {
                    exprs.insert(*k, e);
                }
            }
            let now = self.now();
            let mut rng = self.rng;
            let tp = prof_now();
            let (results, spills, cycles) = {
                let mut host = PassHost::new(wb, &exprs, &mut rng, now, self.hooks.clone(), total);
                host.iterate = wb.calc.iterative;
                for (i, k) in dirty.iter().enumerate() {
                    host.compute(*k);
                    if host.aborted {
                        return false;
                    }
                    let Some(hooks) = self.hooks.as_ref() else { continue };
                    if hooks.cancelled() {
                        return false;
                    }
                    // Fine-grained progress between rounds, so a long pass reports steadily.
                    // First formula + every 4096 thereafter (was only 4096 — huge workbooks
                    // looked stuck at 0 % through the opening batch).
                    if i == 0 || (i + 1).is_multiple_of(4096) {
                        hooks.report((done + i + 1).min(total), total);
                    }
                }
                done += dirty.len();
                if let Some(hooks) = self.hooks.as_ref() {
                    hooks.report(done.min(total), total);
                }
                if host.iterate && !host.members.is_empty() {
                    let passes = host.iterate_loops(&dirty, wb.calc.max_iterations, wb.calc.max_change);
                    if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
                        eprintln!("  iterative calculation: {} passes over {} formulas", passes, host.tainted.len());
                    }
                }
                (host.results, host.spills, host.cycles)
            };
            self.rng = rng;
            if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
                eprintln!("  eval {} formulas {:.1} ms", results.len(), prof_now() - tp);
            }
            let tw = prof_now();
            self.last_recalc_cells = results.len();
            self.circular = cycles;
            // Write back.
            let mut spill_changes: Vec<Key> = Vec::new();
            let mut freed: Vec<(usize, RangeRef)> = Vec::new();
            for (k, v) in results {
                let Some(sheet) = wb.sheet_mut(k.0) else { continue };
                if let Some(cell) = sheet.cells.get_mut(k.1)
                    && cell.value != v
                {
                    cell.value = v;
                }
            }
            for k in &dirty {
                let new = spills.get(k);
                let Some(sheet) = wb.sheet_mut(k.0) else { continue };
                let old = sheet.spill_ranges.get(&k.1).copied();
                if old.is_none() && new.is_none() {
                    continue;
                }
                if let Some(r) = old {
                    for c in r.iter() {
                        sheet.spill.remove(&c);
                    }
                    sheet.spill_ranges.remove(&k.1);
                }
                let mut changed_range = old;
                if let Some(arr) = new {
                    let end = CellRef::new(k.1.row.saturating_add(arr.rows as u32 - 1), k.1.col.saturating_add(arr.cols as u32 - 1));
                    let range = RangeRef::new(k.1, end);
                    let blocked = end.row >= gridcraft_core::MAX_ROWS
                        || end.col >= gridcraft_core::MAX_COLS
                        || range.iter().any(|c| c != k.1 && (sheet.cells.has(c) || sheet.spill.contains_key(&c)))
                        || sheet.merges.iter().any(|m| m.intersects(&range))
                        || sheet.tables.iter().any(|t| t.range.contains(k.1));
                    if blocked {
                        if let Some(cell) = sheet.cells.get_mut(k.1) {
                            cell.value = Value::Error(CellError::Spill);
                        }
                    } else {
                        for (i, c) in range.iter().enumerate() {
                            if c != k.1 {
                                sheet.spill.insert(c, arr.data.get(i).cloned().unwrap_or_default());
                            }
                        }
                        sheet.spill_ranges.insert(k.1, range);
                        changed_range = Some(changed_range.map_or(range, |o| o.union(&range)));
                    }
                }
                if let Some(r) = changed_range {
                    let mut deps = Vec::new();
                    self.graph.dependents_of_range(k.0, r, &mut deps);
                    spill_changes.extend(deps.into_iter().filter(|d| d != k));
                }
                if let Some(o) = old
                    && !sheet.spill_ranges.get(&k.1).is_some_and(|n| n.contains(o.start) && n.contains(o.end))
                {
                    freed.push((k.0, o));
                }
            }
            // Formulas with references only known while evaluating (INDIRECT, OFFSET) may have read
            // a spill area before it was laid out: they are evaluated once more.
            if dynamic_again && !spills.is_empty() {
                dynamic_again = false;
                spill_changes.extend(self.graph.dynamic.iter().copied());
            }
            // An area a formula no longer spills over may unblock another formula's array.
            for (si, o) in freed {
                for &(s, c) in self.graph.nodes.keys() {
                    if s == si
                        && c.row <= o.end.row
                        && c.col <= o.end.col
                        && wb.sheet(s).is_some_and(|sh| sh.value(c) == Value::Error(CellError::Spill))
                    {
                        spill_changes.push((s, c));
                    }
                }
            }
            if full {
                // Everything was already evaluated once; spill dependents need another pass.
            }
            if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
                eprintln!("  write-back {:.1} ms", prof_now() - tw);
            }
            if let Some(sh) = scope_sheet {
                spill_changes.retain(|k| k.0 == sh);
            }
            spill_changes.sort_by_key(|(s, c)| (*s, c.row, c.col));
            spill_changes.dedup();
            if spill_changes.is_empty() {
                break;
            }
            dirty = self.dirty_closure(wb, &spill_changes);
            if let Some(sh) = scope_sheet {
                dirty.retain(|k| k.0 == sh);
            }
        }
        true
    }
}

/// Current date-time as a serial (1900 system), from the system clock (browser clock on wasm).
/// UTC; the local offset isn't available without platform calls. Falls back to a fixed date only
/// when no clock is available.
pub fn now_serial() -> f64 {
    gridcraft_core::date::unix_now_ms().map_or(46302.5, gridcraft_core::date::serial_from_unix_ms)
}

/// Evaluates a formula in the context of a cell without storing it (conditional formats, data
/// validation, Evaluate Formula, Name Manager previews, the CLI `eval`).
pub fn evaluate(wb: &Workbook, sheet: usize, at: CellRef, formula: &str) -> Value {
    let body = formula.strip_prefix('=').unwrap_or(formula);
    let Ok(expr) = gridcraft_formula::parse(body) else { return Value::Error(CellError::Name) };
    evaluate_expr(wb, sheet, at, &expr)
}

pub fn evaluate_expr(wb: &Workbook, sheet: usize, at: CellRef, expr: &Expr) -> Value {
    let exprs = HashMap::default();
    let mut rng = 0x1234_5678_9ABC_DEF0u64;
    let mut host = PassHost::new(wb, &exprs, &mut rng, now_serial(), None, 1);
    let mut ev = Evaluator::new(&mut host, sheet, at);
    ev.value(expr)
}

#[allow(clippy::disallowed_methods)] // the clock is read only off wasm
fn prof_now() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::{Duration, Instant};

    use gridcraft_core::{CellRef, RangeRef, Value};
    use gridcraft_model::{Cell, Formula, Workbook};

    use super::{Calc, HashMap, Host, Intervals, Key, PassHost};

    /// xorshift64 for repeatable pseudo-random cases.
    fn rng(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    fn found(iv: &Intervals<u32>, a: u32, b: u32) -> Vec<u32> {
        let mut v = Vec::new();
        iv.query(a, b, &mut |t| v.push(*t));
        v.sort_unstable();
        v
    }

    #[test]
    fn intervals_match_a_linear_scan() {
        let mut iv: Intervals<u32> = Intervals::default();
        let mut model: Vec<(u32, u32, u32)> = Vec::new();
        let mut s = 0x9E37_79B9_7F4A_7C15;
        for step in 0..20_000u32 {
            let roll = rng(&mut s) % 10;
            if roll < 6 || model.is_empty() {
                let r0 = (rng(&mut s) % 500) as u32;
                let r1 = r0 + (rng(&mut s) % 40) as u32;
                if !model.contains(&(r0, r1, step)) {
                    iv.insert(r0, r1, step);
                    model.push((r0, r1, step));
                }
            } else {
                let i = (rng(&mut s) % model.len() as u64) as usize;
                let (r0, r1, t) = model.swap_remove(i);
                iv.remove(r0, r1, t);
                if roll == 9 {
                    // Removed and put back before the next merge.
                    iv.insert(r0, r1, t);
                    model.push((r0, r1, t));
                }
            }
            if rng(&mut s).is_multiple_of(50) {
                iv.settle();
            }
            if step.is_multiple_of(97) {
                let a = (rng(&mut s) % 560) as u32;
                let b = a + (rng(&mut s) % 30) as u32;
                let mut want: Vec<u32> = model.iter().filter(|(r0, r1, _)| *r0 <= b && *r1 >= a).map(|(_, _, t)| *t).collect();
                want.sort_unstable();
                assert_eq!(found(&iv, a, b), want, "rows {a}..={b} at step {step}");
            }
        }
        iv.settle();
        for row in 0..560 {
            let mut want: Vec<u32> = model.iter().filter(|(r0, r1, _)| *r0 <= row && row <= *r1).map(|(_, _, t)| *t).collect();
            want.sort_unstable();
            assert_eq!(found(&iv, row, row), want, "row {row}");
        }
        for (r0, r1, t) in std::mem::take(&mut model) {
            iv.remove(r0, r1, t);
        }
        iv.settle();
        assert!(iv.is_empty());
    }

    #[test]
    fn interval_queries_cost_log_n_plus_matches() {
        // 40k one-row ranges (like =SUM(E1:J1) down a column) and 1k ranges over everything:
        // a point query visits O(log n) tree nodes per match, not every range in the column.
        let n = 40_000u32;
        let mut iv: Intervals<u32> = Intervals::default();
        for r in 0..n {
            iv.insert(r, r, r);
        }
        for t in 0..1_000 {
            iv.insert(0, n, n + t);
        }
        iv.settle();
        let mut worst = 0;
        for row in (0..n).step_by(7) {
            let mut k = 0;
            let visited = iv.query(row, row, &mut |_| k += 1);
            assert_eq!(k, 1_001);
            worst = worst.max(visited);
        }
        // 1,001 matches in a tree of depth 16: well under 4 nodes per match plus the path.
        assert!(worst < 4 * 1_001 + 64, "visited {worst} nodes");
        let mut k = 0;
        let visited = iv.query(n + 1, n + 5, &mut |_| k += 1);
        assert_eq!(k, 0);
        assert!(visited < 64, "visited {visited} nodes for no match");
    }

    /// `rows` rows of six numbers (E:J) and `=SUM(E{r}:J{r})` in K.
    fn sum_rows(rows: u32) -> Workbook {
        let mut wb = Workbook::new();
        if let Some(sh) = wb.sheet_mut(0) {
            for r in 0..rows {
                for c in 4..10 {
                    sh.cells.set(CellRef::new(r, c), Cell::value(Value::Number(f64::from(r % 97 + c))));
                }
                sh.cells.set(CellRef::new(r, 10), Cell::formula(Formula::new(&format!("=SUM(E{0}:J{0})", r + 1))));
            }
        }
        wb
    }

    /// Opening (full recalc), changing every value of a column, and re-entering every formula.
    #[allow(clippy::disallowed_methods)] // a native-only timing test: the clock is read off wasm
    fn open_and_bulk_edit(rows: u32) -> Duration {
        let mut wb = sum_rows(rows);
        let mut calc = Calc::new();
        let t = Instant::now();
        calc.recalc_all(&mut wb);
        let values: Vec<Key> = (0..rows).map(|r| (0, CellRef::new(r, 4))).collect();
        if let Some(sh) = wb.sheet_mut(0) {
            for (_, c) in &values {
                sh.cells.set(*c, Cell::value(Value::Number(1.0)));
            }
        }
        calc.cells_changed(&mut wb, &values);
        let formulas: Vec<Key> = (0..rows).map(|r| (0, CellRef::new(r, 10))).collect();
        calc.cells_changed(&mut wb, &formulas);
        let elapsed = t.elapsed();
        let last = wb.sheet(0).map(|s| s.value(CellRef::new(rows - 1, 10)));
        let r = rows - 1;
        assert_eq!(last, Some(Value::Number(f64::from(1 + (5..10).map(|c| r % 97 + c).sum::<u32>()))));
        elapsed
    }

    #[test]
    fn range_formulas_recalc_and_bulk_edit_correctly() {
        // The values behind the timing test below, checked on every run.
        open_and_bulk_edit(500);
    }

    #[test]
    #[ignore = "timing-sensitive; run with --ignored in release"]
    #[allow(clippy::disallowed_methods)]
    fn recalc_with_range_formulas_scales_linearly() {
        // Each row's SUM reads a range in the same columns. Recalculating on open used to walk
        // every row per range read (the used range), and finding a cell's dependents scanned
        // every range in its column: 4x the rows took 16x the time. Linear is 4x; allow 8x.
        let best = |rows| (0..3).map(|_| open_and_bulk_edit(rows)).min().unwrap_or_default();
        let small = best(5_000);
        let large = best(20_000);
        assert!(large < small * 8 + Duration::from_millis(100), "5k rows {small:?}, 20k rows {large:?}");
    }

    #[test]
    fn range_dependents_after_edits() {
        // Formulas that read ranges, wide ranges and whole columns, found again after edits.
        let mut wb = Workbook::new();
        let mut calc = Calc::new();
        let set = |wb: &mut Workbook, calc: &mut Calc, at: &str, input: &str| {
            let c = CellRef::parse(at).unwrap();
            let cell = if input.starts_with('=') { Cell::formula(Formula::new(input)) } else { Cell::value(Value::Number(input.parse().unwrap())) };
            wb.sheet_mut(0).unwrap().cells.set(c, cell);
            calc.cells_changed(wb, &[(0, c)]);
        };
        let num = |wb: &Workbook, at: &str| {
            let v = wb.sheet(0).unwrap().value(CellRef::parse(at).unwrap());
            v.as_f64().unwrap_or_else(|| panic!("{at} = {v:?}"))
        };
        set(&mut wb, &mut calc, "A1", "1");
        set(&mut wb, &mut calc, "B2", "2");
        set(&mut wb, &mut calc, "CZ3", "3");
        set(&mut wb, &mut calc, "DA5", "=SUM(A1:B2)");
        set(&mut wb, &mut calc, "DA6", "=SUM(A1:CZ3)");
        set(&mut wb, &mut calc, "DA7", "=SUM(B:B)");
        set(&mut wb, &mut calc, "DA8", "=SUM(2:2)");
        assert_eq!((num(&wb, "DA5"), num(&wb, "DA6"), num(&wb, "DA7"), num(&wb, "DA8")), (3.0, 6.0, 2.0, 2.0));
        set(&mut wb, &mut calc, "B2", "20");
        assert_eq!((num(&wb, "DA5"), num(&wb, "DA6"), num(&wb, "DA7"), num(&wb, "DA8")), (21.0, 24.0, 20.0, 20.0));
        // Re-entering a formula with another range drops the old one.
        set(&mut wb, &mut calc, "DA5", "=SUM(A1:A2)");
        set(&mut wb, &mut calc, "B2", "5");
        assert_eq!(num(&wb, "DA5"), 1.0);
        let mut deps = Vec::new();
        calc.graph.dependents(0, CellRef::parse("B2").unwrap(), &mut deps);
        deps.sort_unstable();
        let at = |s: &str| (0, CellRef::parse(s).unwrap());
        assert_eq!(deps, vec![at("DA6"), at("DA7"), at("DA8")]);
        deps.clear();
        calc.graph.dependents_of_range(0, RangeRef::new(CellRef::parse("A1").unwrap(), CellRef::parse("A9").unwrap()), &mut deps);
        deps.sort_unstable();
        deps.dedup();
        assert_eq!(deps, vec![at("DA5"), at("DA6"), at("DA8")]);
        set(&mut wb, &mut calc, "CZ3", "30");
        assert_eq!(num(&wb, "DA6"), 36.0);
        // Ranges on another sheet.
        wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Data")));
        set(&mut wb, &mut calc, "DA9", "=SUM(Data!A1:A3)+SUM(Data!A:CZ)");
        assert_eq!(num(&wb, "DA9"), 0.0);
        wb.sheet_mut(1).unwrap().cells.set(CellRef::parse("A2").unwrap(), Cell::value(Value::Number(4.0)));
        calc.cells_changed(&mut wb, &[(1, CellRef::parse("A2").unwrap())]);
        assert_eq!(num(&wb, "DA9"), 8.0);
        deps.clear();
        calc.graph.dependents(1, CellRef::parse("A2").unwrap(), &mut deps);
        assert_eq!(deps, vec![at("DA9"), at("DA9")]);
    }

    /// For tests of `PassHost::visit_range`, which can't be reached from outside this module: the
    /// values it hands over for `range` while the formulas in `pending` (cell, formula) have not been
    /// calculated yet, or `None` when it declines the block.
    pub(crate) fn probe_visit_range(wb: &Workbook, pending: &[(&str, &str)], range: RangeRef) -> Option<Vec<Value>> {
        let mut exprs = HashMap::default();
        for (at, formula) in pending {
            let expr = gridcraft_formula::parse(formula.strip_prefix('=').unwrap_or(formula)).ok()?;
            exprs.insert((0, CellRef::parse(at)?), std::sync::Arc::new(expr));
        }

        // A cell in `pending` is calculated the first time something reads it, which is the state
        // of a real pass partway through; `visit_range` has to cope with the formulas it reads
        // being calculated under it.
        let mut rng = 1u64;
        let mut host = PassHost::new(wb, &exprs, &mut rng, 0.0, None, 1);

        let mut seen = Vec::new();
        host.visit_range(0, range, &mut |v| {
            seen.push(v.clone());
            true
        })
        .then_some(seen)
    }
}

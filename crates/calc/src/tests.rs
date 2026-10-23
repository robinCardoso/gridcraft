use std::sync::{Arc, Mutex};

use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_model::{Cell, Formula, Workbook};

use crate::{Calc, CalcHooks};

fn c(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}

struct T {
    wb: Workbook,
    calc: Calc,
}

impl T {
    fn new() -> T {
        T { wb: Workbook::new(), calc: Calc::new() }
    }
    fn set(&mut self, at: &str, input: &str) {
        let cell = if input.starts_with('=') {
            Cell::formula(Formula::new(input))
        } else {
            Cell::value(gridcraft_core::parse::parse_input(input, self.wb.date_system).value)
        };
        self.wb.sheet_mut(0).unwrap().cells.set(c(at), cell);
        self.calc.cells_changed(&mut self.wb, &[(0, c(at))]);
    }
    fn get(&self, at: &str) -> Value {
        self.wb.sheet(0).unwrap().value(c(at))
    }
    fn num(&self, at: &str) -> f64 {
        self.get(at).as_f64().unwrap_or_else(|| panic!("{at} = {:?}", self.get(at)))
    }
}

#[test]
fn spill_range_operator() {
    let mut t = T::new();
    t.set("A1", "3");
    t.set("B1", "=SEQUENCE(A1)");
    t.set("C1", "=SUM(B1#)");
    t.set("C2", "=ROWS(B1#)");
    assert_eq!((t.num("C1"), t.num("C2")), (6.0, 3.0));
    // Growing and shrinking the spill recalculates its users.
    t.set("A1", "5");
    assert_eq!((t.num("C1"), t.num("C2")), (15.0, 5.0));
    t.set("A1", "2");
    assert_eq!((t.num("C1"), t.num("C2")), (3.0, 2.0));
    // A blocked spill, or a cell that doesn't spill, gives #REF!.
    t.set("B2", "x");
    assert_eq!(t.get("C1"), Value::Error(CellError::Ref));
    t.set("B2", "");
    assert_eq!(t.num("C1"), 3.0);
    t.set("D1", "=SUM(A1#)");
    assert_eq!(t.get("D1"), Value::Error(CellError::Ref));
    // Through a defined name.
    t.wb.names.push(gridcraft_model::DefinedName {
        name: "Seq".into(),
        scope: None,
        formula: "Sheet1!$B$1#".into(),
        comment: String::new(),
        hidden: false,
    });
    t.set("E1", "=SUM(Seq)*10");
    assert_eq!(t.num("E1"), 30.0);
    t.set("A1", "4");
    assert_eq!(t.num("E1"), 100.0);
}

#[test]
fn arithmetic_and_dependencies() {
    let mut t = T::new();
    t.set("A1", "2");
    t.set("A2", "3");
    t.set("A3", "=A1*A2+1");
    assert_eq!(t.num("A3"), 7.0);
    t.set("A1", "10");
    assert_eq!(t.num("A3"), 31.0);
    t.set("B1", "=SUM(A1:A3)");
    assert_eq!(t.num("B1"), 44.0);
    t.set("A2", "0");
    assert_eq!(t.num("A3"), 1.0);
    assert_eq!(t.num("B1"), 11.0);
}

#[test]
fn chains_out_of_order() {
    let mut t = T::new();
    t.set("A5", "=A4+1");
    t.set("A4", "=A3+1");
    t.set("A3", "=A2+1");
    t.set("A2", "=A1+1");
    t.set("A1", "1");
    assert_eq!(t.num("A5"), 5.0);
}

#[test]
fn xlookup_returns_a_reference() {
    let mut t = T::new();
    for (i, (k, v)) in [("a", 1), ("b", 2), ("c", 3), ("d", 4)].iter().enumerate() {
        t.set(&format!("A{}", i + 1), k);
        t.set(&format!("B{}", i + 1), &v.to_string());
    }
    t.set("D1", "=SUM(XLOOKUP(\"b\",A1:A4,B1:B4):XLOOKUP(\"d\",A1:A4,B1:B4))");
    assert_eq!(t.num("D1"), 9.0);
    t.set("D2", "=SUM(B1:XLOOKUP(\"c\",A1:A4,B1:B4))");
    assert_eq!(t.num("D2"), 6.0);
    t.set("D3", "=CELL(\"address\",XLOOKUP(\"c\",A1:A4,B1:B4))");
    assert_eq!(t.get("D3"), Value::from("$B$3"));
    // Not found: if_not_found, else #N/A, as before.
    t.set("D4", "=XLOOKUP(\"z\",A1:A4,B1:B4,\"none\")");
    assert_eq!(t.get("D4"), Value::from("none"));
    t.set("D5", "=XLOOKUP(\"z\",A1:A4,B1:B4)");
    assert_eq!(t.get("D5"), Value::Error(CellError::NA));
    // A match on an empty cell is a blank reference: ISBLANK is TRUE and & adds nothing (#374).
    t.set("A5", "e");
    t.set("D6", "=ISBLANK(XLOOKUP(\"e\",A1:A5,B1:B5))");
    assert_eq!(t.get("D6"), Value::Bool(true));
    t.set("D7", "=\"x\"&XLOOKUP(\"e\",A1:A5,B1:B5)&\"y\"");
    assert_eq!(t.get("D7"), Value::from("xy"));
    // An empty match_mode keeps the search_mode after it: search last to first.
    t.set("A6", "b");
    t.set("B6", "20");
    t.set("D8", "=XLOOKUP(\"b\",A1:A6,B1:B6,,,-1)");
    assert_eq!(t.num("D8"), 20.0);
    t.set("D9", "=XLOOKUP(\"b\",A1:A6,B1:B6)");
    assert_eq!(t.num("D9"), 2.0);
}

#[test]
fn long_chain() {
    let mut t = T::new();
    t.set("A1", "1");
    for r in 2..=3000 {
        t.set(&format!("A{r}"), &format!("=A{}+1", r - 1));
    }
    assert_eq!(t.num("A3000"), 3000.0);
    t.set("A1", "10");
    assert_eq!(t.num("A3000"), 3009.0);
}

#[test]
fn index_with_arrays_of_positions() {
    let mut t = T::new();
    for (i, row) in [[1, 2, 3], [4, 5, 6], [7, 8, 9]].iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            t.set(&format!("{}{}", ["A", "B", "C"][j], i + 1), &v.to_string());
        }
    }
    t.set("E1", "=INDEX(A1:C3,{1;3},{1,3})");
    assert_eq!([t.num("E1"), t.num("F1"), t.num("E2"), t.num("F2")], [1.0, 3.0, 7.0, 9.0]);
    t.set("E4", "=SUM(INDEX(A1:C3,{1,2,3},2))");
    assert_eq!(t.num("E4"), 15.0);
    t.set("E5", "=INDEX({10,20,30},{3,1})");
    assert_eq!([t.num("E5"), t.num("F5")], [30.0, 10.0]);
    t.set("E7", "=INDEX(A1:C3,2,3)");
    assert_eq!(t.num("E7"), 6.0);
}

#[test]
fn index_area_number_overflow() {
    let mut t = T::new();
    t.set("A1", "7");
    t.set("D1", "9");
    for (area, expected) in [("1", 7.0), ("2", 9.0)] {
        t.set("G5", &format!("=INDEX((A1:B2,D1:E2),1,1,{area})"));
        assert_eq!(t.num("G5"), expected, "area {area}");
    }
    for area in ["3", "1E300", "-1", "-1E300"] {
        t.set("G5", &format!("=INDEX((A1:B2,D1:E2),1,1,{area})"));
        assert_eq!(t.get("G5"), Value::Error(CellError::Ref), "area {area}");
    }
}

#[test]
fn errors_and_cycles() {
    let mut t = T::new();
    t.set("A1", "=1/0");
    assert_eq!(t.get("A1"), Value::Error(CellError::Div0));
    t.set("B1", "=B2");
    t.set("B2", "=B1");
    assert!(t.get("B1").is_error());
    t.set("C1", "=NOSUCHFN(1)");
    assert_eq!(t.get("C1"), Value::Error(CellError::Name));
    // `_xludf.` marks a user-defined function, never the built-in of the same name (#726).
    for f in ["=_xludf.CONCAT(\"a\",\"b\")", "=_xludf.SUM(1,2)", "=_xludf.EOMONTH()"] {
        t.set("C3", f);
        assert_eq!(t.get("C3"), Value::Error(CellError::Name), "{f}");
    }
    t.set("C2", "=1+");
    assert_eq!(t.get("C2"), Value::Error(CellError::Name));
}

impl T {
    fn iterative(max_iterations: u32, max_change: f64) -> T {
        let mut t = T::new();
        t.wb.calc.iterative = true;
        t.wb.calc.set_max_iterations(max_iterations);
        t.wb.calc.set_max_change(max_change);
        t
    }
}

#[test]
fn iterative_calculation_stops_at_maximum_iterations() {
    // A1 =A1+1 never settles: every recalculation adds Maximum Iterations.
    let mut t = T::iterative(100, 0.001);
    t.set("A1", "=A1+1");
    assert_eq!(t.num("A1"), 100.0);
    assert!(t.calc.circular.is_empty());
    // An edit outside the loop doesn't recalculate it.
    t.set("B1", "7");
    assert_eq!(t.num("A1"), 100.0);
    // Each recalculation starts from the value the cell shows.
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.num("A1"), 200.0);
    t.wb.calc.set_max_iterations(5);
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.num("A1"), 205.0);
    // Through a range that holds the formula itself.
    t.set("C1", "1");
    t.set("C2", "2");
    t.set("C3", "=SUM(C1:C3)");
    assert_eq!(t.num("C3"), 15.0);
    // Excel's largest Maximum Iterations.
    let mut t = T::iterative(32767, 0.001);
    t.set("A1", "=A1+1");
    assert_eq!(t.num("A1"), 32767.0);
}

#[test]
fn iterative_calculation_converges_within_maximum_change() {
    let mut t = T::iterative(100, 0.001);
    t.set("A1", "=(A2+10)/2");
    t.set("A2", "=A1");
    assert!((t.num("A1") - 10.0).abs() < 0.002, "{}", t.num("A1"));
    assert!((t.num("A2") - 10.0).abs() < 0.002, "{}", t.num("A2"));
    assert!(t.calc.circular.is_empty());
    // A loop fed by a constant, and a formula that reads the loop, follow a change.
    t.set("D1", "10");
    t.set("A1", "=(A2+D1)/2");
    t.set("E1", "=A1*2");
    t.set("D1", "30");
    assert!((t.num("A1") - 30.0).abs() < 0.002, "{}", t.num("A1"));
    assert!((t.num("E1") - 60.0).abs() < 0.004, "{}", t.num("E1"));
    // Already settled: recalculating changes it by less than Maximum Change.
    let a1 = t.num("A1");
    t.calc.recalc_all(&mut t.wb);
    assert!((t.num("A1") - a1).abs() < 0.001);
}

#[test]
fn iterative_calculation_stops_once_changes_are_below_maximum_change() {
    // A1 halves its distance to 10 each pass, from 5: 7.5, 8.75, 9.375, 9.6875, and A2 trails
    // one pass behind. Maximum Change 1 stops once neither moved by 1 or more.
    let mut t = T::iterative(100, 1.0);
    t.set("A1", "=(A2+10)/2");
    assert_eq!(t.num("A1"), 5.0);
    t.set("A2", "=A1");
    assert_eq!((t.num("A1"), t.num("A2")), (9.6875, 9.375));
    // Maximum Iterations 2 stops first.
    let mut t = T::iterative(2, 1.0);
    t.set("A1", "=(A2+10)/2");
    t.set("A2", "=A1");
    assert_eq!((t.num("A1"), t.num("A2")), (8.75, 7.5));
}

#[test]
fn iterative_calculation_of_text_and_errors() {
    let mut t = T::iterative(100, 0.001);
    t.set("A1", "=IF(ISTEXT(A1),A1&\"\",\"go\")");
    assert_eq!(t.get("A1"), Value::from("go"));
    t.set("B1", "=B1+1/0");
    assert_eq!(t.get("B1"), Value::Error(CellError::Div0));
}

#[test]
fn circular_references_without_iterative_calculation() {
    let mut t = T::new();
    t.set("A1", "=A1+1");
    assert_eq!(t.get("A1"), Value::Error(CellError::Circ));
    assert_eq!(t.calc.circular, vec![(0, c("A1"))]);
    // Switched on, the loop is calculated from 0 rather than from the error.
    t.wb.calc.iterative = true;
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.num("A1"), 100.0);
    assert!(t.calc.circular.is_empty());
    t.wb.calc.iterative = false;
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.get("A1"), Value::Error(CellError::Circ));
}

#[test]
fn date_text_coerces_in_the_workbook_date_system() {
    for (sys, serial) in [(gridcraft_core::DateSystem::D1900, 1462.0), (gridcraft_core::DateSystem::D1904, 0.0)] {
        let mut t = T::new();
        t.wb.date_system = sys;
        t.set("A1", "'1/1/1904");
        t.set("B1", "=A1+0");
        t.set("C1", "=VALUE(A1)");
        t.set("D1", "=-A1");
        t.set("E1", "=DATEVALUE(A1)");
        t.set("F1", "=NUMBERVALUE(A1)");
        assert_eq!(t.get("A1"), Value::from("1/1/1904"));
        assert_eq!(t.num("B1"), serial, "{sys:?}");
        assert_eq!(t.num("C1"), serial, "{sys:?}");
        assert_eq!(t.num("D1"), -serial, "{sys:?}");
        assert_eq!(t.num("E1"), serial, "{sys:?}");
        assert_eq!(t.num("F1"), serial, "{sys:?}");
    }
}

#[test]
fn operators() {
    let mut t = T::new();
    t.set("A1", "=\"a\"&1&TRUE");
    assert_eq!(t.get("A1"), Value::from("a1TRUE"));
    t.set("A2", "=-2^2");
    assert_eq!(t.num("A2"), 4.0);
    t.set("A3", "=50%");
    assert_eq!(t.num("A3"), 0.5);
    t.set("A4", "=\"abc\"=\"ABC\"");
    assert_eq!(t.get("A4"), Value::Bool(true));
    t.set("A5", "=\"5\"+1");
    assert_eq!(t.num("A5"), 6.0);
    t.set("A6", "=\"x\"+1");
    assert_eq!(t.get("A6"), Value::Error(CellError::Value));
    t.set("A7", "=B99");
    assert_eq!(t.num("A7"), 0.0);
}

#[test]
fn blocked_spill_reads_as_spill_error() {
    // Calculated in the same pass, a formula over a blocked spill read the array that couldn't
    // spill.
    let mut t = T::new();
    let s = t.wb.sheet_mut(0).unwrap();
    for a in ["A1", "B1", "C1", "A2", "C2", "A3", "B3", "C3"] {
        s.cells.set(c(a), Cell::value(Value::Number(1.0)));
    }
    s.cells.set(c("B2"), Cell::formula(Formula::new("=SEQUENCE(2,2,9)")));
    s.cells.set(c("E1"), Cell::formula(Formula::new("=SUM(A1:C3)")));
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.get("B2"), Value::Error(CellError::Spill));
    assert_eq!(t.get("E1"), Value::Error(CellError::Spill));
    assert_eq!(t.num("C3"), 1.0);
}

#[test]
fn blocked_spill_read_through_offset() {
    // The reader's reference is only known while evaluating, so it isn't in the dependency graph.
    let mut t = T::new();
    t.set("E4", "5");
    t.set("E8", "=SUM(OFFSET(E2,0,0,2,2))");
    t.set("E3", "=SEQUENCE(2,2,3)");
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.get("E3"), Value::Error(CellError::Spill));
    assert_eq!(t.get("E8"), Value::Error(CellError::Spill));
}

#[test]
fn spill_into_the_old_area_of_a_blocked_neighbour() {
    // C7's array becomes blocked by C8, whose array then takes over C7's old area.
    let mut t = T::new();
    t.set("C7", "=SEQUENCE(2,2)");
    t.set("C8", "=SEQUENCE(2,2)");
    assert_eq!(t.get("C7"), Value::Error(CellError::Spill));
    assert_eq!([t.num("C8"), t.num("D8"), t.num("C9"), t.num("D9")], [1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn arrays_do_not_spill_in_tables() {
    // Excel: spilled array formulas aren't supported in tables.
    let mut t = T::new();
    t.wb.sheet_mut(0).unwrap().tables.push(gridcraft_model::Table {
        id: 1,
        name: "Table1".into(),
        range: gridcraft_core::RangeRef::parse("B3:C6").unwrap(),
        header_row: true,
        totals_row: false,
        columns: vec![],
        style: String::new(),
        banded_rows: false,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: false,
        filter: vec![],
        table_type: None,
        connection_id: None,
    });
    t.set("C4", "=SEQUENCE(2)");
    t.set("E1", "=SEQUENCE(4)");
    t.set("H1", "=SUM(C4:C5)");
    assert_eq!(t.get("C4"), Value::Error(CellError::Spill));
    assert_eq!(t.get("C5"), Value::Empty);
    assert_eq!(t.get("H1"), Value::Error(CellError::Spill));
    assert_eq!(t.num("E4"), 4.0);
}

#[test]
fn ifs_and_lazy() {
    let mut t = T::new();
    t.set("A1", "5");
    t.set("B1", "=IF(A1>3,\"big\",1/0)");
    assert_eq!(t.get("B1"), Value::from("big"));
    t.set("B2", "=IFERROR(1/0,\"oops\")");
    assert_eq!(t.get("B2"), Value::from("oops"));
    t.set("B3", "=CHOOSE(2,\"a\",\"b\",\"c\")");
    assert_eq!(t.get("B3"), Value::from("b"));
    t.set("B4", "=SWITCH(A1,1,\"one\",5,\"five\",\"other\")");
    assert_eq!(t.get("B4"), Value::from("five"));
}

#[test]
fn choose_with_an_array_index() {
    let mut t = T::new();
    for (i, (k, v)) in [(1, 10), (2, 20), (3, 30)].iter().enumerate() {
        t.set(&format!("A{}", i + 1), &k.to_string());
        t.set(&format!("B{}", i + 1), &v.to_string());
    }
    t.set("D1", "=CHOOSE({1,2},\"a\",\"b\")");
    assert_eq!(t.get("D1"), Value::from("a"));
    assert_eq!(t.get("E1"), Value::from("b"));
    // The usual "look left" idiom: a two-column array from two ranges.
    t.set("D3", "=VLOOKUP(20,CHOOSE({1,2},B1:B3,A1:A3),2,FALSE)");
    assert_eq!(t.num("D3"), 2.0);
    t.set("D4", "=SUM(CHOOSE({1,2},A1:A3,B1:B3))");
    assert_eq!(t.num("D4"), 66.0);
    t.set("D5", "=CHOOSE(2,A1:A3,B1:B3)");
    assert_eq!(t.num("D7"), 30.0);
}

#[test]
fn vlookup_and_hlookup_on_range_references() {
    let mut t = T::new();
    t.set("A1", "Code");
    t.set("B1", "Name");
    t.set("C1", "Price");

    t.set("A2", "10023AN");
    t.set("B2", "JUNTA");
    t.set("C2", "7.76");

    t.set("A3", "60660orsc");
    t.set("B3", "JG.JTAS.");
    t.set("C3", "807.48");

    // Exact match VLOOKUP on full column or block range
    t.set("E1", "=VLOOKUP(\"10023AN\", A1:C3, 2, FALSE)");
    assert_eq!(t.get("E1"), Value::from("JUNTA"));

    t.set("E2", "=VLOOKUP(\"60660orsc\", A1:C3, 3, FALSE)");
    assert_eq!(t.num("E2"), 807.48);

    // Case-insensitive exact match
    t.set("E3", "=VLOOKUP(\"60660ORSC\", A1:C3, 2, FALSE)");
    assert_eq!(t.get("E3"), Value::from("JG.JTAS."));

    // Not found returns #N/A
    t.set("E4", "=VLOOKUP(\"MISSING\", A1:C3, 2, FALSE)");
    assert_eq!(t.get("E4"), Value::Error(CellError::NA));

    // Column out of bounds returns #REF!
    t.set("E5", "=VLOOKUP(\"10023AN\", A1:C3, 4, FALSE)");
    assert_eq!(t.get("E5"), Value::Error(CellError::Ref));

    // HLOOKUP
    t.set("E6", "=HLOOKUP(\"Price\", A1:C3, 2, FALSE)");
    assert_eq!(t.num("E6"), 7.76);
}

#[test]
fn references_functions() {
    let mut t = T::new();
    for (i, v) in [10, 20, 30, 40].iter().enumerate() {
        t.set(&format!("A{}", i + 1), &v.to_string());
    }
    t.set("B1", "=ROW(A3)");
    assert_eq!(t.num("B1"), 3.0);
    t.set("B2", "=ROWS(A1:A4)*COLUMNS(A1:C1)");
    assert_eq!(t.num("B2"), 12.0);
    t.set("B3", "=SUM(OFFSET(A1,1,0,2,1))");
    assert_eq!(t.num("B3"), 50.0);
    t.set("B4", "=INDIRECT(\"A\"&4)");
    assert_eq!(t.num("B4"), 40.0);
    // With a1 = FALSE the text must be R1C1: A1-style text is #REF!, not read as A1.
    t.set("C1", "=INDIRECT(\"R3C1\",FALSE)");
    assert_eq!(t.num("C1"), 30.0);
    t.set("C2", "=INDIRECT(\"A3\",FALSE)");
    assert_eq!(t.get("C2"), Value::Error(CellError::Ref));
    // Defined names don't depend on the reference style.
    t.wb.names.push(gridcraft_model::DefinedName {
        name: "Third".into(),
        scope: None,
        formula: "Sheet1!$A$3".into(),
        comment: String::new(),
        hidden: false,
    });
    t.calc.rebuild(&t.wb);
    t.set("C3", "=INDIRECT(\"Third\",FALSE)");
    assert_eq!(t.num("C3"), 30.0);
    t.set("B5", "=SUM(A1:INDEX(A1:A4,3))");
    assert_eq!(t.num("B5"), 60.0);
    t.set("B6", "=SUM(A:A)");
    assert_eq!(t.num("B6"), 100.0);
    t.set("A2", "25");
    assert_eq!(t.num("B3"), 55.0);
    assert_eq!(t.num("B6"), 105.0);
}

#[test]
fn indirect_r1c1_out_of_bounds() {
    for reference in ["R[9223372036854775807]C", "RC[9223372036854775807]", "R[4294967296]C", "R[4294967291]C", "R[2000000]C", "RC[20000]"] {
        let mut t = T::new();
        t.set("D5", &format!("=ROW(INDIRECT(\"{reference}\",FALSE))"));
        assert_eq!(t.get("D5"), Value::Error(CellError::Ref), "{reference}");
    }
}

#[test]
fn indirect_r1c1_past_the_sheet_does_not_fall_back_to_a1() {
    let mut t = T::new();
    t.set("RC16385", "78");
    for reference in ["RC16385", "rc16385", "RC1:RC16385"] {
        t.set("D5", &format!("=INDIRECT(\"{reference}\",FALSE)"));
        assert_eq!(t.get("D5"), Value::Error(CellError::Ref), "{reference}");
    }
}

#[test]
fn indirect_r1c1_valid_references() {
    let mut t = T::new();
    t.set("E6", "11");
    t.set("A1", "22");
    t.set("D4", "33");
    for (reference, expected) in [("R[1]C[1]", 11.0), ("R1C1", 22.0), ("R[-1]C", 33.0)] {
        t.set("D5", &format!("=INDIRECT(\"{reference}\",FALSE)"));
        assert_eq!(t.num("D5"), expected, "{reference}");
    }
}

#[test]
fn relative_names() {
    let mut t = T::new();
    // As files store them: relative to A1. `Left` is the cell to the left, `Above3` the three
    // cells above, `Mixed` column A of the same row.
    for (name, formula) in [("Left", "Sheet1!XFD1"), ("Above3", "Sheet1!A1048574:A1048576"), ("Mixed", "Sheet1!$A1")] {
        t.wb.names.push(gridcraft_model::DefinedName {
            name: name.into(),
            scope: None,
            formula: formula.into(),
            comment: String::new(),
            hidden: false,
        });
    }
    t.calc.rebuild(&t.wb);
    for r in 1..=5 {
        t.set(&format!("A{r}"), &(r * 10).to_string());
    }
    t.set("B2", "=Left");
    assert_eq!(t.num("B2"), 20.0);
    t.set("B4", "=Left+Mixed");
    assert_eq!(t.num("B4"), 80.0);
    t.set("A6", "=SUM(Above3)");
    assert_eq!(t.num("A6"), 120.0);
}

#[test]
fn spills() {
    let mut t = T::new();
    t.set("A1", "=SEQUENCE(3)");
    assert_eq!(t.num("A1"), 1.0);
    assert_eq!(t.num("A3"), 3.0);
    t.set("B1", "=SUM(A1:A3)");
    assert_eq!(t.num("B1"), 6.0);
    t.set("A2", "x");
    assert_eq!(t.get("A1"), Value::Error(CellError::Spill));
    t.set("A2", "");
    assert_eq!(t.num("A3"), 3.0);
    t.set("C1", "={1,2;3,4}*10");
    assert_eq!(t.num("D2"), 40.0);
}

#[test]
fn references_keep_their_size_past_the_used_range() {
    let mut t = T::new();
    t.set("A1", "5");
    t.set("A2", "6");
    t.set("C1", "=ROWS(A1:A10*1)");
    t.set("C2", "=SUMPRODUCT(--(A1:A10=\"\"))");
    t.set("C3", "=COUNTBLANK(A1:A10)");
    t.set("C4", "=COUNTIF(A1:A10,\"\")");
    t.set("C5", "=COLUMNS(TRANSPOSE(A1:A10))");
    for (at, want) in [("C1", 10.0), ("C2", 8.0), ("C3", 8.0), ("C4", 8.0), ("C5", 10.0)] {
        assert_eq!(t.num(at), want, "{at}");
    }
    // Whole columns are still read up to the used range.
    t.set("D1", "=ROWS(FILTER(A:A,A:A<>\"\"))");
    assert_eq!(t.num("D1"), 2.0);
}

#[test]
fn spilled_blank_cells_show_zero() {
    let mut t = T::new();
    t.set("A1", "5");
    t.set("A3", "7");
    t.set("C1", "=A1:A4");
    assert_eq!(t.get("C2"), Value::Number(0.0));
    assert_eq!(t.get("C4"), Value::Number(0.0));
    t.set("D1", "=COUNT(C1:C4)");
    assert_eq!(t.num("D1"), 4.0);
    t.set("E1", "=A2:A3");
    assert_eq!(t.get("E1"), Value::Number(0.0));
    t.set("F1", "=SORT(A1:A3)");
    assert_eq!(t.get("F3"), Value::Number(0.0));
}

#[test]
fn let_lambda() {
    let mut t = T::new();
    t.set("A1", "=LET(x,2,y,3,x*y)");
    assert_eq!(t.num("A1"), 6.0);
    t.set("A2", "=LAMBDA(a,b,a+b)(2,3)");
    assert_eq!(t.num("A2"), 5.0);
    t.set("A3", "=REDUCE(0,{1,2,3},LAMBDA(acc,v,acc+v))");
    assert_eq!(t.num("A3"), 6.0);
    t.set("A4", "=SUM(MAP({1,2,3},LAMBDA(v,v*v)))");
    assert_eq!(t.num("A4"), 14.0);
}

/// Two rows with the same lookup key must resolve to the same MATCH index (and thus the same
/// INDEX description). Guards against LET/`val_cache` or ThisRow bugs that only show up when
/// keys collide rather than when every row differs.
#[test]
fn same_lookup_key_two_rows_same_match_index() {
    let mut t = T::new();
    t.set("A1", "MB001");
    t.set("A2", "MB006");
    t.set("A3", "MB003");
    t.set("B1", "Alpha");
    t.set("B2", "Bravo");
    t.set("B3", "Charlie");
    let f1 = concat!(
        "=LET(codigo,C1,",
        "LET(achou,A$1:A$3=codigo,IF(SUM(--achou)=0,0,MATCH(TRUE,achou,0))))",
    );
    let f2 = concat!(
        "=LET(codigo,C2,",
        "LET(achou,A$1:A$3=codigo,IF(SUM(--achou)=0,0,MATCH(TRUE,achou,0))))",
    );
    t.set("C1", "MB006");
    t.set("C2", "MB006");
    t.set("D1", f1);
    t.set("D2", f2);
    t.set("E1", "=IF(D1>0,INDEX(B$1:B$3,D1),\"\")");
    t.set("E2", "=IF(D2>0,INDEX(B$1:B$3,D2),\"\")");
    assert_eq!(t.num("D1"), 2.0);
    assert_eq!(t.num("D2"), 2.0, "identical keys must share the first-match index");
    assert_eq!(t.get("E1"), Value::from("Bravo"));
    assert_eq!(t.get("E2"), Value::from("Bravo"));
}

/// Nested LET must not cache a binding that only names an outer LET variable: the AST looks
/// invariant (no relative ref / ThisRow), but the outer binding can differ per cell. Without
/// seeding `variant_names` from `env`, PassHost's `val_cache` reused the first row's array
/// (COTACAO `Linha` / INDEX all resolving as MB001).
#[test]
fn nested_let_does_not_reuse_outer_binding_across_rows() {
    let mut t = T::new();
    // Lookup column: row 1 → 10, row 2 → 20, row 3 → 30
    t.set("A1", "MB001");
    t.set("A2", "MB002");
    t.set("A3", "MB003");
    t.set("B1", "10");
    t.set("B2", "20");
    t.set("B3", "30");
    // Same shape as COTACAO Linha: outer LET binds codigo from this row; inner LET builds
    // `achou` from a whole-column compare against codigo, then MATCH.
    let f = concat!(
        "=LET(codigo,A1,",
        "LET(achou,A$1:A$3=codigo,IF(SUM(--achou)=0,0,MATCH(TRUE,achou,0))))",
    );
    t.set("C1", f);
    t.set("C2", &f.replace("A1", "A2"));
    t.set("C3", &f.replace("A1", "A3"));
    assert_eq!(t.num("C1"), 1.0);
    assert_eq!(t.num("C2"), 2.0, "inner LET must recompute achou for this row's codigo");
    assert_eq!(t.num("C3"), 3.0);
}

#[test]
fn functions_given_by_name() {
    let mut t = T::new();
    for (i, (k, v)) in [("a", 1), ("b", -2), ("a", 3)].iter().enumerate() {
        t.set(&format!("A{}", i + 1), k);
        t.set(&format!("B{}", i + 1), &v.to_string());
    }
    t.set("D1", "=BYROW(B1:B3,SUM)");
    assert_eq!(t.num("D2"), -2.0);
    t.set("E1", "=MAP(B1:B3,ABS)");
    assert_eq!(t.num("E2"), 2.0);
    t.set("F1", "=REDUCE(0,B1:B3,SUM)");
    assert_eq!(t.num("F1"), 2.0);
    t.set("G1", "=GROUPBY(A1:A3,B1:B3,SUM,0,0)");
    assert_eq!(t.get("G1"), Value::from("a"));
    assert_eq!(t.num("H1"), 4.0);
    // A LET variable of that name is still the variable.
    t.set("J1", "=LET(ABS,LAMBDA(x,x*10),SUM(MAP(B1:B3,ABS)))");
    assert_eq!(t.num("J1"), 20.0);
}

#[test]
fn lambda_argument_count() {
    let mut t = T::new();
    t.wb.names.push(gridcraft_model::DefinedName {
        name: "AddPair".into(),
        scope: None,
        formula: "LAMBDA(a,b,a+b)".into(),
        comment: String::new(),
        hidden: false,
    });
    t.calc.rebuild(&t.wb);
    let omit = "LAMBDA(x,y,IF(ISOMITTED(y),\"Missing\",x+y))";
    t.set("A1", "=LAMBDA(a,b,a+b)(2)");
    t.set("A2", "=LAMBDA(a,b,a+b)(2,3,4)");
    t.set("A3", &format!("={omit}(1,)"));
    t.set("A4", &format!("={omit}(1)"));
    t.set("A5", &format!("={omit}(1,2)"));
    t.set("A6", "=LAMBDA(7)()");
    t.set("A7", "=AddPair(2)");
    t.set("A8", "=AddPair(2,3)");
    assert_eq!(t.get("A1"), Value::Error(CellError::Value));
    assert_eq!(t.get("A2"), Value::Error(CellError::Value));
    assert_eq!(t.get("A3"), Value::Text("Missing".into()));
    assert_eq!(t.get("A4"), Value::Error(CellError::Value));
    assert_eq!(t.num("A5"), 3.0);
    assert_eq!(t.num("A6"), 7.0);
    assert_eq!(t.get("A7"), Value::Error(CellError::Value));
    assert_eq!(t.num("A8"), 5.0);
    t.set("A9", "=LAMBDA(x,ISOMITTED(x))()");
    assert_eq!(t.get("A9"), Value::Bool(true));
}

#[test]
fn names_and_sheets() {
    let mut t = T::new();
    t.wb.names.push(gridcraft_model::DefinedName { name: "Rate".into(), scope: None, formula: "0.5".into(), comment: String::new(), hidden: false });
    t.wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Data")));
    t.wb.sheet_mut(1).unwrap().set_value(c("A1"), Value::Number(8.0));
    t.calc.rebuild(&t.wb);
    t.set("A1", "=Rate*Data!A1");
    assert_eq!(t.num("A1"), 4.0);
    t.wb.sheet_mut(1).unwrap().set_value(c("A1"), Value::Number(10.0));
    t.calc.cells_changed(&mut t.wb, &[(1, c("A1"))]);
    assert_eq!(t.num("A1"), 5.0);
}

#[test]
fn legacy_array_formulas_fill_their_range() {
    let mut t = T::new();
    t.set("A1", "1");
    t.set("A2", "2");
    let cse = |t: &mut T, at: &str, formula: &str, range: &str| {
        let mut f = Formula::new(formula);
        f.array = gridcraft_core::RangeRef::parse(range);
        t.wb.sheet_mut(0).unwrap().cells.set(c(at), Cell::formula(f));
        t.calc.cells_changed(&mut t.wb, &[(0, c(at))]);
    };
    // Past the result: #N/A. A single column repeats across the range.
    cse(&mut t, "C1", "=A1:A2*2", "C1:D3");
    cse(&mut t, "F1", "=A1:A2*2", "F1");
    cse(&mut t, "G1", "=5", "G1:G2");
    assert_eq!(t.num("C2"), 4.0);
    assert_eq!(t.num("D2"), 4.0);
    assert_eq!(t.get("C3"), Value::Error(CellError::NA));
    // A smaller range shows only part of the result and never spills.
    assert_eq!(t.num("F1"), 2.0);
    assert_eq!(t.get("F2"), Value::Empty);
    assert_eq!(t.num("G2"), 5.0);
    // A range too large to fill is an error, not an allocation of every cell.
    cse(&mut t, "H1", "=1", "H1:XFD1048576");
    assert_eq!(t.get("H1"), Value::Error(CellError::Num));
}

#[test]
fn text_function() {
    let mut t = T::new();
    t.set("A1", "=TEXT(1234.5,\"#,##0.00\")");
    assert_eq!(t.get("A1"), Value::from("1,234.50"));
}

#[test]
fn long_operator_chains() {
    let mut t = T::new();
    t.set("A1", "1");
    // 1,000 additions: a formula of 3,000 characters, well within Excel's 8,192.
    t.set("B1", &format!("={}", vec!["A1"; 1000].join("+")));
    assert_eq!(t.num("B1"), 1000.0);
    t.set("B2", &format!("=5{}", "+1-1".repeat(600)));
    assert_eq!(t.num("B2"), 5.0);
}

#[test]
fn subtotal_skips_hidden() {
    let mut t = T::new();
    t.set("A1", "1");
    t.set("A2", "2");
    t.set("A3", "3");
    t.wb.sheet_mut(0).unwrap().rows.insert(1, gridcraft_model::LineInfo { hidden: true, ..Default::default() });
    t.set("B1", "=SUBTOTAL(109,A1:A3)");
    t.set("B2", "=SUBTOTAL(9,A1:A3)");
    assert_eq!(t.num("B1"), 4.0);
    assert_eq!(t.num("B2"), 6.0);
}

#[test]
fn subtotal_109_skips_nested_subtotal() {
    let mut t = T::new();
    t.set("A1", "2");
    t.set("A2", "=SUBTOTAL(9,A1)");
    t.set("C1", "=SUBTOTAL(109,A1:A2)");
    t.set("C2", "=SUBTOTAL(9,A1:A2)");
    t.set("C3", "=SUBTOTAL(109,A1)");
    assert_eq!(t.num("C1"), 2.0);
    assert_eq!(t.num("C2"), 2.0);
    assert_eq!(t.num("C3"), 2.0);
    // Manually hidden rows are still left out by 109 only.
    let mut t = T::new();
    t.set("A1", "1");
    t.set("A2", "2");
    t.set("A3", "=SUBTOTAL(9,A1:A2)");
    t.wb.sheet_mut(0).unwrap().rows.insert(1, gridcraft_model::LineInfo { hidden: true, ..Default::default() });
    t.set("B1", "=SUBTOTAL(109,A1:A3)");
    t.set("B2", "=SUBTOTAL(9,A1:A3)");
    assert_eq!(t.num("B1"), 1.0);
    assert_eq!(t.num("B2"), 3.0);
}

/// SUBTOTAL leaves out rows a filter hides for every function number, while 1-11 keep rows
/// hidden by hand (as in ConditionalFormattingSamples.xlsx, where `SUBTOTAL(9,Table69[Sales])`
/// under a filtered table adds only the visible rows).
#[test]
fn subtotal_skips_filtered_rows() {
    use gridcraft_model::{AutoFilter, FilterCriterion, LineInfo};
    let mut t = T::new();
    for (i, v) in [1, 2, 4, 8, 16, 32].iter().enumerate() {
        t.set(&format!("A{}", i + 2), &v.to_string());
        t.set(&format!("C{}", i + 2), &v.to_string());
    }
    let crit = || vec![(0, FilterCriterion::Custom { a: (">".into(), "1".into()), b: None, and: false })];
    let sh = t.wb.sheet_mut(0).unwrap();
    sh.tables.push(gridcraft_model::Table {
        id: 1,
        name: "Sales".into(),
        range: gridcraft_core::RangeRef::parse("A1:A7").unwrap(),
        header_row: true,
        totals_row: false,
        columns: vec![gridcraft_model::TableColumn {
            name: "Amount".into(),
            totals: Default::default(),
            totals_label: None,
            formula: None,
            unique_name: None,
            xml: None,
        }],
        style: String::new(),
        banded_rows: false,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: true,
        filter: crit(),
        table_type: None,
        connection_id: None,
    });
    sh.autofilter = Some(AutoFilter { range: gridcraft_core::RangeRef::parse("C1:C7").unwrap(), criteria: crit() });
    // Rows 3 and 5 are filtered out; row 9 (outside both filters) is hidden by hand.
    for row in [2, 4, 8] {
        sh.rows.insert(row, LineInfo { hidden: true, ..Default::default() });
    }
    t.set("A9", "64");
    t.set("E1", "=SUBTOTAL(9,Sales[Amount])");
    t.set("E2", "=SUBTOTAL(109,Sales[Amount])");
    t.set("E3", "=SUBTOTAL(9,C2:C7)");
    t.set("E4", "=SUBTOTAL(9,A2:A9)");
    t.set("E5", "=SUBTOTAL(109,A2:A9)");
    t.set("E6", "=SUBTOTAL(2,A2:A7)");
    t.set("E7", "=AGGREGATE(9,4,A2:A7)");
    for (at, want) in [("E1", 53.0), ("E2", 53.0), ("E3", 53.0), ("E4", 117.0), ("E5", 53.0), ("E6", 4.0), ("E7", 63.0)] {
        assert_eq!(t.num(at), want, "{at}");
    }
}

#[test]
fn aggregate_without_data_argument_is_an_error() {
    let mut t = T::new();
    t.set("A1", "1");
    t.set("A2", "2");
    t.set("D1", "=AGGREGATE(9,0)");
    t.set("D2", "=AGGREGATE(4,0)");
    t.set("D3", "=AGGREGATE(9,0,A1:A2)");
    t.set("D4", "=AGGREGATE(9,0,A3:A4)");
    t.set("D5", "=AGGREGATE(9,0,A1:A2,A1:A2)");
    assert_eq!(t.get("D1"), Value::Error(CellError::Value));
    assert_eq!(t.get("D2"), Value::Error(CellError::Value));
    assert_eq!(t.num("D3"), 3.0);
    assert_eq!(t.num("D4"), 0.0);
    assert_eq!(t.num("D5"), 6.0);
}

#[test]
fn recalc_all_after_load() {
    let mut t = T::new();
    let s = t.wb.sheet_mut(0).unwrap();
    s.cells.set(c("A1"), Cell::value(Value::Number(4.0)));
    s.cells.set(c("A2"), Cell::formula(Formula::new("=A1*2")));
    s.cells.set(c("A3"), Cell::formula(Formula::new("=A2+A1")));
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.num("A3"), 12.0);
}

#[test]
fn sumif_sum_range_takes_the_criteria_size() {
    // Excel adds up the range that starts at sum_range's top-left cell with the criteria range's
    // height and width, whatever size sum_range has.
    let mut t = T::new();
    for (r, (k, v)) in [("a", 1), ("b", 2), ("a", 3)].iter().enumerate() {
        t.set(&format!("A{}", r + 1), k);
        t.set(&format!("B{}", r + 1), &v.to_string());
    }
    t.set("D1", "=SUMIF(A1:A3,\"a\",B1)");
    assert_eq!(t.num("D1"), 4.0);
    t.set("D2", "=AVERAGEIF(A1:A3,\"a\",B1)");
    assert_eq!(t.num("D2"), 2.0);
    t.set("D3", "=SUMIF(A1:A3,\"a\",B1:B2)");
    assert_eq!(t.num("D3"), 4.0);
    t.set("D4", "=SUMIF(A1:A3,\"a\",B1:F1)");
    assert_eq!(t.num("D4"), 4.0);
    t.set("D5", "=SUMIF(A1:A3,\"a\",B1:B99)");
    assert_eq!(t.num("D5"), 4.0);
    t.set("D6", "=SUMIF(A:A,\"a\",B1)");
    assert_eq!(t.num("D6"), 4.0);
    t.set("D7", "=SUMIF(A1:A3,\"a\",$B$2)");
    assert_eq!(t.num("D7"), 2.0); // B2:B4: rows 1 and 3 match → B2 + B4 (empty)
    t.set("D8", "=SUMIF(A1:A3,\"a\")");
    assert_eq!(t.num("D8"), 0.0);
    // A horizontal criteria range resizes across.
    t.set("H1", "1");
    t.set("I1", "0");
    t.set("J1", "1");
    t.set("H2", "10");
    t.set("I2", "20");
    t.set("J2", "30");
    t.set("D9", "=SUMIF(H1:J1,1,H2)");
    assert_eq!(t.num("D9"), 40.0);
    // The resized cells are precedents: editing one recalculates.
    t.set("B3", "5");
    assert_eq!(t.num("D1"), 6.0);
    assert_eq!(t.num("D2"), 3.0);
    assert_eq!(t.num("D3"), 6.0);
    assert_eq!(t.num("D4"), 6.0);
    assert_eq!(t.num("D6"), 6.0);
    // SUMIFS and AVERAGEIFS don't resize: different sizes are #VALUE!.
    t.set("D11", "=SUMIFS(B1,A1:A3,\"a\")");
    assert_eq!(t.get("D11"), Value::Error(CellError::Value));
    t.set("D12", "=AVERAGEIFS(B1:B2,A1:A3,\"a\")");
    assert_eq!(t.get("D12"), Value::Error(CellError::Value));
    t.set("D13", "=SUMIFS(B1:B3,A1:A3,\"a\")");
    assert_eq!(t.num("D13"), 6.0);
}

#[test]
fn sumif_resized_sum_range_stops_at_the_sheet_edge() {
    let mut t = T::new();
    t.set("A1", "a");
    t.set("A2", "a");
    t.set("B1048575", "5");
    t.set("B1048576", "7");
    t.set("C1", "=SUMIF(A1:A2,\"a\",B1048576)");
    assert_eq!(t.num("C1"), 7.0);
    t.set("C2", "=SUMIF(A1:A2,\"a\",B1048575)");
    assert_eq!(t.num("C2"), 12.0);
    t.set("C3", "=SUMIF(A1:B1,\"a\",XFD1)");
    assert_eq!(t.num("C3"), 0.0);
}

#[test]
fn sumif_resized_sum_range_on_another_sheet_and_through_a_name() {
    let mut t = T::new();
    t.wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Data")));
    t.wb.names.push(gridcraft_model::DefinedName {
        name: "Amounts".into(),
        scope: None,
        formula: "Data!$B$1".into(),
        comment: String::new(),
        hidden: false,
    });
    for (r, k) in ["a", "b", "a"].iter().enumerate() {
        t.set(&format!("A{}", r + 1), k);
        t.wb.sheet_mut(1).unwrap().set_value(CellRef::new(r as u32, 1), Value::Number((r + 1) as f64));
    }
    t.calc.rebuild(&t.wb);
    t.set("C1", "=SUMIF(A1:A3,\"a\",Data!B1)");
    assert_eq!(t.num("C1"), 4.0);
    t.set("C2", "=AVERAGEIF(A1:A3,\"a\",Amounts)");
    assert_eq!(t.num("C2"), 2.0);
    t.wb.sheet_mut(1).unwrap().set_value(c("B3"), Value::Number(9.0));
    t.calc.cells_changed(&mut t.wb, &[(1, c("B3"))]);
    assert_eq!(t.num("C1"), 10.0);
    assert_eq!(t.num("C2"), 5.0);
}

#[test]
fn trimrange() {
    let mut t = T::new();
    t.set("C3", "1");
    t.set("D5", "2");
    let cases = [
        ("J1", "=ROWS(TRIMRANGE(A1:F10))", 3.0),
        ("J2", "=COLUMNS(TRIMRANGE(A1:F10))", 2.0),
        ("J3", "=ROWS(TRIMRANGE(A1:F10,2))", 5.0),
        ("J4", "=ROWS(TRIMRANGE(A1:F10,1,0))", 8.0),
        ("J5", "=COLUMNS(TRIMRANGE(A1:F10,1,0))", 6.0),
        ("J6", "=ROWS(TRIMRANGE(A1:F10,0,0))", 10.0),
        ("J7", "=SUM(TRIMRANGE(C:C))", 1.0),
        ("J8", "=ROWS(TRIMRANGE(C:C))", 1.0),
        ("J9", "=ROWS(TRIMRANGE(C:C,2))", 3.0),
        ("J10", "=MIN(ROW(TRIMRANGE(A1:F10)))", 3.0),
        ("J11", "=COLUMNS(TRIMRANGE({1,2;3,4}))", 2.0),
    ];
    for (at, f, _) in cases {
        t.set(at, f);
    }
    for (at, f, want) in cases {
        assert_eq!(t.num(at), want, "{f}");
    }
    // The result spills as the trimmed block and follows edits.
    t.set("L1", "=TRIMRANGE(A1:F10)");
    assert_eq!((t.num("L1"), t.num("M3")), (1.0, 2.0));
    t.set("E6", "3");
    assert_eq!((t.num("J1"), t.num("J2")), (4.0, 3.0));
    // Errors: a bad trim code, nothing but blanks, several areas.
    t.set("K1", "=TRIMRANGE(A1:F10,4)");
    assert_eq!(t.get("K1"), Value::Error(CellError::Value));
    t.set("K2", "=TRIMRANGE(Q1:Q9)");
    assert_eq!(t.get("K2"), Value::Error(CellError::Calc));
    t.set("K3", "=TRIMRANGE((A1:B2,C3:D4))");
    assert_eq!(t.get("K3"), Value::Error(CellError::Value));
}

/// `formula` in AB1 over a small mixed sheet. The ranges in the cases below are cut from it:
///
/// ```text
///      A      B      C       E
///  1   1.5    10     =A1*4        (C1 is a formula cell inside the ranges)
///  2   2      TRUE           #DIV/0!   (E2 only when `error` is set)
///  3   text   3
///  4   -0.25
/// ```
fn aggregate_of(error: bool, formula: &str) -> Value {
    let mut t = T::new();
    for (at, v) in [("A1", "1.5"), ("A2", "2"), ("A3", "skipped"), ("A4", "-0.25"), ("B1", "10"), ("B2", "TRUE"), ("B3", "3"), ("C1", "=A1*4")] {
        t.set(at, v);
    }
    if error {
        t.set("E2", "#DIV/0!");
    }

    // AB1 is outside every range used, so the formula can't read itself.
    t.set("AB1", formula);
    t.get("AB1")
}

#[test]
fn sum_count_average_over_ranges() {
    // Text, booleans, blanks and a formula cell inside ranges; direct arguments; unions; whole
    // columns; blocks holding an error or nothing numeric. The expected values are what the
    // array path (the ranges copied into an array) gave before the ranges were folded in place,
    // so any difference is a change of behavior.
    let n = Value::Number;
    let e = Value::Error;

    // (formula written with SUM, error in E2?, [SUM, COUNT, AVERAGE] of that formula)
    #[rustfmt::skip]
    let cases: &[(&str, bool, [Value; 3])] = &[
        ("=SUM(A1:A1,B3)", false, [n(4.5), n(2.0), n(2.25)]),
        ("=SUM(A1:A1,B3)", true, [n(4.5), n(2.0), n(2.25)]),
        ("=SUM((A1:A2,B1:B3))", false, [n(16.5), n(4.0), n(4.125)]),
        ("=SUM((A1:A2,B1:B3))", true, [n(16.5), n(4.0), n(4.125)]),
        ("=SUM(A1:A4,B1:B3)", false, [n(16.25), n(5.0), n(3.25)]),
        ("=SUM(A1:A4,B1:B3)", true, [n(16.25), n(5.0), n(3.25)]),
        ("=SUM(A1:A4,\"x\")", false, [e(CellError::Value), n(3.0), e(CellError::Value)]),
        ("=SUM(A1:A4,\"x\")", true, [e(CellError::Value), n(3.0), e(CellError::Value)]),
        ("=SUM(A1:B4,5,\"7\",TRUE)", false, [n(29.25), n(8.0), n(3.65625)]),
        ("=SUM(A1:B4,5,\"7\",TRUE)", true, [n(29.25), n(8.0), n(3.65625)]),
        ("=SUM(A1:C4)+SUM(B1:B3)", false, [n(35.25), n(8.0), n(10.208333333333334)]),
        ("=SUM(A1:C4)+SUM(B1:B3)", true, [n(35.25), n(8.0), n(10.208333333333334)]),
        ("=SUM(A1:C4)", false, [n(22.25), n(6.0), n(3.7083333333333335)]),
        ("=SUM(A1:C4)", true, [n(22.25), n(6.0), n(3.7083333333333335)]),
        ("=SUM(A1:E4)", false, [n(22.25), n(6.0), n(3.7083333333333335)]),
        ("=SUM(A1:E4)", true, [e(CellError::Div0), n(6.0), e(CellError::Div0)]),
        ("=SUM(A1)", false, [n(1.5), n(1.0), n(1.5)]),
        ("=SUM(A1)", true, [n(1.5), n(1.0), n(1.5)]),
        ("=SUM(A3,B2)", false, [n(0.0), n(0.0), e(CellError::Div0)]),
        ("=SUM(A3,B2)", true, [n(0.0), n(0.0), e(CellError::Div0)]),
        ("=SUM(A:C)", false, [n(22.25), n(6.0), n(3.7083333333333335)]),
        ("=SUM(A:C)", true, [n(22.25), n(6.0), n(3.7083333333333335)]),
        ("=SUM(E1:E3)", false, [n(0.0), n(0.0), e(CellError::Div0)]),
        ("=SUM(E1:E3)", true, [e(CellError::Div0), n(0.0), e(CellError::Div0)]),
        ("=SUM(Z1:Z9)", false, [n(0.0), n(0.0), e(CellError::Div0)]),
        ("=SUM(Z1:Z9)", true, [n(0.0), n(0.0), e(CellError::Div0)]),
    ];

    for (formula, error, want) in cases {
        for (func, want) in ["SUM", "COUNT", "AVERAGE"].into_iter().zip(want) {
            // The same formula for each function: `SUM(` becomes `COUNT(` or `AVERAGE(`.
            let f = formula.replace("SUM", func);

            assert_eq!(&aggregate_of(*error, &f), want, "{f} (error in E2: {error})");
        }
    }
}

#[test]
fn sum_count_average_have_no_size_limit() {
    // One stored cell far down stretches the used range to 27 million cells (27 columns by a
    // million rows), past the 16M-cell limit of ranges copied into an array. Only two cells are
    // stored, so the sheet itself is tiny.
    for (func, want) in [("SUM", Value::Number(2.5)), ("COUNT", Value::Number(2.0)), ("AVERAGE", Value::Number(1.25))] {
        for range in ["A1:AA1000000", "A:AA"] {
            let mut t = T::new();
            t.set("A1", "1.5");
            t.set("Z1000000", "1");

            t.set("AB1", &format!("={func}({range})"));

            assert_eq!(t.get("AB1"), want, "{func}({range})");
        }
    }
}

#[test]
fn sum_count_average_over_spilled_values() {
    // A1 spills over A1:A3, so the block holds values that exist only as a spill (they have no
    // stored cell of their own). The host must not fold such a block in place: it falls back
    // to the array path, which sees the spill. Expected: 1 + 2 + 3 + 10.
    let mut t = T::new();
    t.set("A1", "=SEQUENCE(3)");
    t.set("A4", "10");

    for (f, want) in [("SUM", 16.0), ("COUNT", 4.0), ("AVERAGE", 4.0)] {
        t.set("C1", &format!("={f}(A1:A5)"));

        assert_eq!(t.num("C1"), want, "{f}");
    }
}

#[test]
fn aggregate_arguments_are_read_in_order() {
    // Volatile cells draw a random number each time they are calculated, so the order in which
    // the arguments are read decides which draw each one gets. `B1:B2` comes first, so B2 must
    // draw before the `RAND()` argument does. (Reading the plain `RAND()` first once gave
    // 7.9043 here.) The expected values are what the array path gave; they depend on the random
    // generator and its seed, and must be updated if either changes.
    let mut t = T::new();
    t.set("B1", "1");
    t.set("B2", "=RAND()*10");

    t.set("A1", "=SUM(B1:B2,RAND())");

    assert_eq!((t.num("A1"), t.num("B2")), (4.968520165759784, 3.3112028100185356));
}

#[test]
fn sum_count_average_follow_edits_inside_the_range() {
    // A range read in place must still count as a precedent of the formula: if it didn't, an
    // edit inside it would leave the formula showing a stale value. Each step below changes one
    // cell inside the range and checks all three functions; the last one changes a cell outside.
    for range in ["A1:A5", "A:A"] {
        let mut t = T::new();
        t.set("A1", "1");
        t.set("A3", "3");
        t.set("C1", &format!("=SUM({range})"));
        t.set("C2", &format!("=COUNT({range})"));
        t.set("C3", &format!("=AVERAGE({range})"));
        let all = |t: &T| (t.num("C1"), t.num("C2"), t.num("C3"));
        assert_eq!(all(&t), (4.0, 2.0, 2.0), "{range}");

        t.set("A1", "5");
        assert_eq!(all(&t), (8.0, 2.0, 4.0), "{range}: edit");

        t.set("A2", "10");
        assert_eq!(all(&t), (18.0, 3.0, 6.0), "{range}: fill a blank");

        t.set("A3", "text");
        assert_eq!(all(&t), (15.0, 2.0, 7.5), "{range}: number to text");

        t.set("A2", "");
        assert_eq!(all(&t), (5.0, 1.0, 5.0), "{range}: clear");

        t.set("A5", "=A1*2");
        assert_eq!(all(&t), (15.0, 2.0, 7.5), "{range}: formula in the range");

        t.set("A1", "1");
        assert_eq!(all(&t), (3.0, 2.0, 1.5), "{range}: its precedent changes");

        // A7 is inside the whole column A:A but outside A1:A5.
        t.set("A7", "100");
        let outside = if range == "A:A" { (103.0, 3.0, 103.0 / 3.0) } else { (3.0, 2.0, 1.5) };
        assert_eq!(all(&t), outside, "{range}: a cell outside the range");
    }
}

#[test]
fn visit_range_declines_when_a_formula_in_the_block_spills_into_it() {
    // The sheet: D1 = SEQUENCE(3) and D4 = 10, and D1 has not been calculated yet when the
    // block D1:D5 is read.
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    sh.cells.set(c("D1"), Cell::formula(Formula::new("=SEQUENCE(3)")));
    sh.cells.set(c("D4"), Cell::value(Value::Number(10.0)));
    let block = gridcraft_core::RangeRef::parse("D1:D5").unwrap();

    // Reading the block calculates D1, which spills over D2:D3. A walk over the stored cells
    // would never see 2 and 3, so the host must decline and leave the block to `range_values`.
    // (`Calc` recalculates a pass that produced new spills, so final results are right either
    // way; only this direct call shows the host's contract.)
    let walked = crate::recalc::tests::probe_visit_range(&wb, &[("D1", "=SEQUENCE(3)")], block);

    assert_eq!(walked, None, "walked a block holding a spill it can't see");
}

/// Trim references evaluate as `TRIMRANGE(range, code, code)`.
#[test]
fn trim_references() {
    let mut t = T::new();
    t.set("C3", "1");
    t.set("D5", "2");
    t.set("A20", "5");
    t.set("A21", "6");
    let cases = [
        ("J1", "=ROWS(A1.:.F10)", "=ROWS(TRIMRANGE(A1:F10,3,3))", 3.0),
        ("J2", "=COLUMNS(A1.:.F10)", "=COLUMNS(TRIMRANGE(A1:F10,3,3))", 2.0),
        ("J3", "=ROWS(A1:.F10)", "=ROWS(TRIMRANGE(A1:F10,2,2))", 5.0),
        ("J4", "=COLUMNS(A1:.F10)", "=COLUMNS(TRIMRANGE(A1:F10,2,2))", 4.0),
        ("J5", "=ROWS(A1.:F10)", "=ROWS(TRIMRANGE(A1:F10,1,1))", 8.0),
        ("J6", "=MIN(ROW(A1.:F10))", "=MIN(ROW(TRIMRANGE(A1:F10,1,1)))", 3.0),
        ("J7", "=SUM(A:.A)", "=SUM(TRIMRANGE(A:A,2,2))", 11.0),
        ("J8", "=ROWS(A:.A)", "=ROWS(TRIMRANGE(A:A,2,2))", 21.0),
        ("J9", "=ROWS(A.:.A)", "=ROWS(TRIMRANGE(A:A,3,3))", 2.0),
        ("J10", "=ROWS(20.:.30)", "=ROWS(TRIMRANGE(20:30,3,3))", 2.0),
    ];
    for (at, f, _, _) in cases {
        t.set(at, f);
    }
    for (i, (at, f, same, want)) in cases.iter().enumerate() {
        assert_eq!(t.num(at), *want, "{f}");
        let k = format!("K{}", i + 1);
        t.set(&k, same);
        assert_eq!(t.num(&k), *want, "{same}");
    }
    // The trimmed block spills and follows edits to the full range.
    t.set("L1", "=C1.:.F10");
    assert_eq!((t.num("L1"), t.num("M3")), (1.0, 2.0));
    t.set("A22", "4");
    assert_eq!((t.num("J7"), t.num("J8")), (15.0, 22.0));
    // Nothing but blanks.
    t.set("K20", "=SUM(Q1:.Q9)");
    assert_eq!(t.get("K20"), Value::Error(CellError::Calc));
}

#[test]
fn cell_width_is_the_column_width_dialog_value_rounded() {
    // #735: Excel 365 values. Widths in px as `home.columnWidth {"chars"}` sets them
    // (chars * 7 + 5): 10.71 -> 80, 12 -> 89, 10.57 -> 79, 2.86 -> 25; B keeps the default 8.43 (64).
    let mut t = T::new();
    let sh = t.wb.sheet_mut(0).unwrap();
    for (col, px) in [(2, 80.0), (3, 89.0), (4, 79.0), (5, 25.0)] {
        sh.cols.insert(col, gridcraft_model::LineInfo { size: Some(px), ..Default::default() });
    }
    for (at, col) in [("H1", "B"), ("H2", "C"), ("H3", "D"), ("H4", "E"), ("H5", "F")] {
        t.set(at, &format!("=CELL(\"width\",{col}1)"));
    }
    assert_eq!([t.num("H1"), t.num("H2"), t.num("H3"), t.num("H4"), t.num("H5")], [8.0, 11.0, 12.0, 11.0, 3.0]);
}

/// Expected results are Excel's, from Apache POI's `DGet.xls` (rows 46-49): a criteria cell holding
/// a formula under a label that names no field is evaluated for every record.
#[test]
fn database_functions_with_computed_criteria() {
    let mut t = T::new();
    let db = [
        ["name", "age", "size", "headgear", "glasses"],
        ["Huey", "6", "45", "cap~", "FALSE"],
        ["Dewey", "7", "fourty-six", "cap?", "FALSE"],
        ["Louie", "8", "47", "cap*", "FALSE"],
        ["Donald", "32", "70", "sailor's hat", "FALSE"],
        ["Scrooge", "62", "65", "top hat", "TRUE"],
    ];
    for (r, row) in db.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            t.set(&format!("{}{}", ["F", "G", "H", "I", "J"][c], r + 2), v);
        }
    }
    t.set("G45", "7");
    let cases = [
        // Relative references are to the first record's row and move to each record's row;
        // `$G$45` stays put.
        ("G", "=G2<$G$45", "Dewey"),
        // Column labels stand for the record's fields.
        ("H", "=glasses=TRUE", "Scrooge"),
        ("I", "=age<7", "Huey"),
        ("J", "=size/7=10", "Donald"),
        // An absolute reference to a cell that is not the first record's.
        ("K", "=H3=$L$1", "Donald"),
    ];
    t.set("L1", "70");
    for (i, (col, formula, _)) in cases.iter().enumerate() {
        t.set(&format!("{col}46"), "Formula");
        t.set(&format!("{col}47"), formula);
        t.set(&format!("B{}", 46 + i), &format!("=DGET($F$2:$J$7,\"name\",{col}46:{col}47)"));
    }
    for (i, (_, formula, want)) in cases.iter().enumerate() {
        assert_eq!(t.get(&format!("B{}", 46 + i)), Value::text(*want), "{formula}");
    }
    // Every database function takes them, ANDed with the other criteria of their row.
    // Text sorts after numbers, so Dewey's "fourty-six" is > 40.
    t.set("L47", "=size>40");
    t.set("M46", "age");
    t.set("M47", ">6");
    t.set("B51", "=DCOUNT($F$2:$J$7,\"age\",I46:I47)");
    t.set("B52", "=DSUM($F$2:$J$7,\"age\",G46:G47)");
    t.set("B53", "=DCOUNTA($F$2:$J$7,\"name\",L46:M47)");
    assert_eq!(t.num("B51"), 1.0);
    assert_eq!(t.num("B52"), 7.0);
    assert_eq!(t.num("B53"), 4.0);
}

#[test]
fn offset_extreme_arguments_return_ref() {
    let wb = Workbook::new();
    for formula in [
        "=OFFSET(B2,1E300,0)",
        "=OFFSET(B2,0,1E300)",
        "=OFFSET(B2,0,0,1E300,1)",
        "=OFFSET(B2,0,0,1,1E300)",
        "=OFFSET(B2,-1E300,0)",
        "=OFFSET(B2,0,-1E300)",
        "=OFFSET(B2,0,0,-1E300,1)",
        "=OFFSET(B2,0,0,1,-1E300)",
        "=OFFSET(B2,1E300,0,-1E300,1)",
        "=OFFSET(B2,0,1E300,1,-1E300)",
        "=OFFSET(B2,-1E300,0,-1E300,1)",
    ] {
        assert_eq!(crate::evaluate(&wb, 0, c("D1"), formula), Value::Error(CellError::Ref), "{formula}");
    }
    for (formula, expected) in [
        ("=ROW(OFFSET(B2,1.9,0))", 3.0),
        ("=COLUMN(OFFSET(B2,0,-1.9))", 1.0),
        ("=ROWS(OFFSET(B2,0,0,-2,1))", 2.0),
        ("=COLUMNS(OFFSET(B2,0,0,1,-2))", 2.0),
        ("=ROWS(OFFSET(A1,0,0,1048576,1))", 1048576.0),
    ] {
        assert_eq!(crate::evaluate(&wb, 0, c("D1"), formula), Value::Number(expected), "{formula}");
    }
}

#[test]
fn median_large_values_recalculate_dependents() {
    let mut t = T::new();
    t.set("A1", "1e308");
    t.set("A2", "1.6e308");
    t.set("B1", "=MEDIAN(A1:A2)");
    t.set("B2", "=AGGREGATE(12,0,A1:A2)");
    t.set("C1", "=B1/1e308");
    t.set("C2", "=B2/1e308");
    for cell in ["B1", "B2"] {
        assert!((t.num(cell) / 1e308 - 1.3).abs() < 1e-14);
    }
    for cell in ["C1", "C2"] {
        assert!((t.num(cell) - 1.3).abs() < 1e-14);
    }
    t.set("A2", "-1e308");
    for cell in ["B1", "B2", "C1", "C2"] {
        assert_eq!(t.num(cell), 0.0);
    }
    t.set("A1", "-1.6e308");
    for cell in ["B1", "B2"] {
        assert!((t.num(cell) / 1e308 + 1.3).abs() < 1e-14);
    }
    for cell in ["C1", "C2"] {
        assert!((t.num(cell) + 1.3).abs() < 1e-14);
    }
}

/// A workbook with `n` independent formulas in column B (an F9-shaped recalculation load).
fn chain(n: u32) -> Workbook {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    sh.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(7.0)));
    for r in 0..n {
        sh.cells.set(CellRef::new(r, 1), Cell::formula(Formula::new("=A1+1")));
    }
    wb
}

#[test]
fn hooks_report_progress_through_a_recalculation() {
    let mut wb = chain(6000);
    let hooks = Arc::new(CalcHooks::new());
    let seen = Arc::new(Mutex::new(Vec::<(usize, usize)>::new()));
    let collect = seen.clone();
    hooks.set_progress(Box::new(move |done, total| {
        if let Ok(mut v) = collect.lock() {
            v.push((done, total));
        }
    }));
    let mut calc = Calc::new();
    calc.set_hooks(hooks);
    calc.recalc_all(&mut wb);
    let reports = seen.lock().unwrap();
    let last = *reports.last().unwrap_or(&(0, 0));
    assert_eq!(last, (6000, 6000), "the final report covers every formula: {:?}", reports.last());
    // Leading (0, _) nudges are expected (ensure_graph / start of run_scoped); a mid-flight
    // report must still show real work before the final total.
    let mid = reports.iter().copied().find(|(done, total)| *done > 0 && *done < *total);
    assert!(mid.is_some(), "in-flight reports grow towards the total: {reports:?}");
    // The result is identical to the same recalculation without hooks (never crash: the hooks
    // are observers only).
    let mut plain = chain(6000);
    let mut calc2 = Calc::new();
    calc2.recalc_all(&mut plain);
    assert_eq!(wb, plain);
}

#[test]
fn hooks_cancel_stop_a_recalculation() {
    let mut wb = chain(500);
    let hooks = Arc::new(CalcHooks::new());
    hooks.cancel();
    let mut calc = Calc::new();
    calc.set_hooks(hooks.clone());
    assert!(!calc.recalc_all(&mut wb), "a pre-cancelled recalc reports incomplete");
    assert!(hooks.cancelled());
    assert_eq!(wb.sheet(0).unwrap().value(c("B1")), Value::Empty, "a cancelled pass writes nothing back");
    assert_eq!(wb.sheet(0).unwrap().value(c("B500")), Value::Empty);
    // Detaching the hooks (a new plain run) still finishes the job.
    let mut calc2 = Calc::new();
    assert!(calc2.recalc_all(&mut wb));
    assert_eq!(wb.sheet(0).unwrap().value(c("B1")), Value::Number(8.0));
}

/// Cancel mid-rebuild must leave the graph invalid and stop without evaluating formulas.
#[test]
fn hooks_cancel_during_rebuild_leaves_graph_invalid() {
    let wb = chain(8_000);
    let hooks = Arc::new(CalcHooks::new());
    let seen = Arc::new(Mutex::new(Vec::<(usize, usize)>::new()));
    let collect = seen.clone();
    let cancel_at = hooks.clone();
    hooks.set_progress(Box::new(move |done, total| {
        if let Ok(mut v) = collect.lock() {
            v.push((done, total));
        }
        // Cancel once rebuild has reported real work (done > 0), before it finishes.
        if done > 0 && done < total {
            cancel_at.cancel();
        }
    }));
    let mut calc = Calc::new();
    calc.set_hooks(hooks.clone());
    assert!(!calc.rebuild(&wb), "cancel during rebuild returns false");
    assert!(!calc.graph_is_current(), "a partial rebuild must not be trusted");
    assert!(hooks.cancelled());
    let reports = seen.lock().unwrap();
    assert!(reports.iter().any(|(d, t)| *d > 0 && *d < *t), "rebuild reported progress before cancel: {reports:?}");
}

/// A live recalc must leave the hard "0 done" nudge: at least one report with done > 0 while
/// hooks are attached (regression for status "1% — 0 / N" stuck forever).
#[test]
fn hooks_progress_advances_past_zero_done() {
    let mut wb = chain(5_000);
    let hooks = Arc::new(CalcHooks::new());
    let seen = Arc::new(Mutex::new(Vec::<(usize, usize)>::new()));
    let collect = seen.clone();
    hooks.set_progress(Box::new(move |done, total| {
        if let Ok(mut v) = collect.lock() {
            v.push((done, total));
        }
    }));
    let mut calc = Calc::new();
    calc.set_hooks(hooks);
    assert!(calc.recalc_all(&mut wb));
    let reports = seen.lock().unwrap();
    assert!(
        reports.iter().any(|(done, _)| *done > 0),
        "progress must leave 0 done while the job is alive: {reports:?}"
    );
}

/// Time-based yield (Q1): with eager coop, progress reports far more often than the 4096-count
/// milestones alone — the bar stays alive even when `done` advances slowly.
#[test]
fn hooks_eager_coop_reports_finer_than_count_milestones() {
    let mut wb = chain(2_000);
    let hooks = Arc::new(CalcHooks::new());
    hooks.set_coop_eager(true);
    let seen = Arc::new(Mutex::new(Vec::<(usize, usize)>::new()));
    let collect = seen.clone();
    hooks.set_progress(Box::new(move |done, total| {
        if let Ok(mut v) = collect.lock() {
            v.push((done, total));
        }
    }));
    let mut calc = Calc::new();
    calc.set_hooks(hooks);
    assert!(calc.recalc_all(&mut wb));
    let reports = seen.lock().unwrap();
    // Count-only path: start nudge + first + every 4096 + final ≈ a handful for 2k formulas.
    // Eager coop reports on every compute entry → hundreds of samples.
    assert!(
        reports.len() > 50,
        "eager coop must report finer than count milestones alone: {} reports",
        reports.len()
    );
}

/// Simulated slow formulas still emit progress often enough that the bar does not look dead (Q1).
#[test]
#[allow(clippy::disallowed_methods)] // native timing assertion for Q1
fn hooks_progress_reports_under_slow_compute() {
    let mut wb = chain(6);
    let hooks = Arc::new(CalcHooks::new());
    hooks.set_coop_interval_ms(15);
    hooks.set_slow_compute_us(30_000); // 30 ms per newly computed formula
    let stamps = Arc::new(Mutex::new(Vec::<std::time::Instant>::new()));
    let collect = stamps.clone();
    hooks.set_progress(Box::new(move |_done, _total| {
        if let Ok(mut v) = collect.lock() {
            v.push(std::time::Instant::now());
        }
    }));
    let mut calc = Calc::new();
    calc.set_hooks(hooks);
    assert!(calc.recalc_all(&mut wb), "slow compute without cancel must finish");
    let stamps = stamps.lock().unwrap();
    assert!(stamps.len() >= 4, "slow pass must report repeatedly: {} stamps", stamps.len());
    let mut max_gap = std::time::Duration::ZERO;
    for w in stamps.windows(2) {
        max_gap = max_gap.max(w[1].saturating_duration_since(w[0]));
    }
    // Aceite do plano: progresso pelo menos a cada ~200 ms (folga de CI).
    assert!(
        max_gap < std::time::Duration::from_millis(250),
        "report gap {max_gap:?} exceeds 250 ms under slow load"
    );
}

/// Cancel during a slow compute aborts without write-back (cooperative path still live).
#[test]
fn hooks_cancel_during_slow_compute() {
    let mut wb = chain(12);
    let hooks = Arc::new(CalcHooks::new());
    hooks.set_coop_interval_ms(10);
    hooks.set_slow_compute_us(40_000);
    let cancel_at = hooks.clone();
    hooks.set_progress(Box::new(move |done, _total| {
        // Wait until real formula work started (not just rebuild nudges at done==0).
        if done >= 1 {
            cancel_at.cancel();
        }
    }));
    let mut calc = Calc::new();
    calc.set_hooks(hooks.clone());
    assert!(!calc.recalc_all(&mut wb), "cancel mid slow compute returns incomplete");
    assert!(hooks.cancelled());
    assert_eq!(wb.sheet(0).unwrap().value(c("B1")), Value::Empty, "cancelled pass writes nothing back");
}

/// Fase 3 (docs/plano-responsividade.md): the per-pass range cache keeps ranges apart — the same
/// coordinates on two sheets are different ranges — and reads the same values as a direct read.
#[test]
fn range_cache_keeps_ranges_apart() {
    let mut wb = Workbook::new();
    wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Other")));
    {
        let a = wb.sheet_mut(0).unwrap();
        a.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(1.0)));
        a.cells.set(CellRef::new(1, 0), Cell::value(Value::Number(2.0)));
        let b = wb.sheet_mut(1).unwrap();
        b.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(10.0)));
        b.cells.set(CellRef::new(1, 0), Cell::value(Value::Number(20.0)));
    }
    let s0 = wb.sheet_mut(0).unwrap();
    s0.cells.set(CellRef::new(0, 1), Cell::formula(Formula::new("=SUM(A1:A2)")));
    s0.cells.set(CellRef::new(1, 1), Cell::formula(Formula::new("=SUM(A1:A2)")));
    s0.cells.set(CellRef::new(2, 1), Cell::formula(Formula::new("=SUM(Other!A1:A2)")));
    let mut calc = Calc::new();
    calc.recalc_all(&mut wb);
    let sh = wb.sheet(0).unwrap();
    assert_eq!(sh.value(CellRef::new(0, 1)), Value::Number(3.0));
    assert_eq!(sh.value(CellRef::new(1, 1)), Value::Number(3.0), "a cache hit returns the same values");
    assert_eq!(sh.value(CellRef::new(2, 1)), Value::Number(30.0), "the same coordinates on another sheet stay distinct");
}

/// Regression: Automatic `cells_changed` must not drop other-sheet dirty cells when the
/// cascade exceeds 1_000 (commit c7a3c16 truncated to the active sheet and left cross-sheet
/// dependents stale — formulas on the active sheet that read them looked static).
#[test]
fn automatic_cascade_keeps_cross_sheet_dependents_past_thousand() {
    let n = 1_100u32;
    let mut t = T::new();
    t.wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Data")));
    t.wb.active_sheet = 0;
    t.wb.sheet_mut(0).unwrap().cells.set(c("A1"), Cell::value(Value::Number(1.0)));
    // Active-sheet formula that reads a cross-sheet dependent (the "static" symptom).
    t.wb.sheet_mut(0).unwrap().cells.set(c("B1"), Cell::formula(Formula::new("=Data!A1")));
    {
        let data = t.wb.sheet_mut(1).unwrap();
        for r in 0..n {
            data.cells.set(CellRef::new(r, 0), Cell::formula(Formula::new("=Sheet1!A1")));
        }
    }
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.wb.sheet(1).unwrap().value(c("A1")), Value::Number(1.0));
    assert_eq!(t.num("B1"), 1.0);

    t.wb.sheet_mut(0).unwrap().cells.set(c("A1"), Cell::value(Value::Number(7.0)));
    t.calc.cells_changed(&mut t.wb, &[(0, c("A1"))]);

    assert_eq!(t.wb.sheet(1).unwrap().value(c("A1")), Value::Number(7.0), "other-sheet dependents update");
    assert_eq!(t.wb.sheet(1).unwrap().value(CellRef::new(n - 1, 0)), Value::Number(7.0));
    assert_eq!(t.num("B1"), 7.0, "active formula reading cross-sheet is not static");
}

/// Manual mode only evaluates the edited formulas themselves — dependents stay until F9.
#[test]
fn manual_cells_changed_does_not_cascade_cross_sheet_dependents() {
    let mut t = T::new();
    t.wb.calc.mode = gridcraft_model::CalcMode::Manual;
    t.wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Data")));
    t.wb.sheet_mut(0).unwrap().cells.set(c("A1"), Cell::value(Value::Number(1.0)));
    t.wb.sheet_mut(1).unwrap().cells.set(c("A1"), Cell::formula(Formula::new("=Sheet1!A1")));
    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.wb.sheet(1).unwrap().value(c("A1")), Value::Number(1.0));

    t.wb.sheet_mut(0).unwrap().cells.set(c("A1"), Cell::value(Value::Number(9.0)));
    t.calc.cells_changed(&mut t.wb, &[(0, c("A1"))]);
    assert_eq!(t.wb.sheet(1).unwrap().value(c("A1")), Value::Number(1.0), "Manual leaves dependents stale");

    t.calc.recalc_all(&mut t.wb);
    assert_eq!(t.wb.sheet(1).unwrap().value(c("A1")), Value::Number(9.0), "F9 pays the debt");
}

/// Fase 3: F9's `ensure_graph` skips a full rebuild when nodes were kept current by
/// `cells_changed` / `update_nodes`; results match a from-scratch rebuild.
#[test]
fn ensure_graph_skips_rebuild_when_current_and_matches_full() {
    let mut wb = chain(200);
    let mut calc = Calc::new();
    calc.recalc_all(&mut wb);
    assert!(calc.graph_is_current());
    let n = calc.graph.len();
    assert_eq!(n, 200);

    // Touch an input: graph stays current (node for A1 removed / absent).
    wb.sheet_mut(0).unwrap().cells.set(c("A1"), Cell::value(Value::Number(3.0)));
    calc.update_nodes(&mut wb, &[(0, c("A1"))]);
    assert!(calc.graph_is_current());
    assert_eq!(calc.graph.len(), n, "a value cell is not a formula node");

    // F9 with a current graph must equal a fresh full rebuild + recalc.
    let mut incremental = wb.clone();
    let mut calc_inc = calc.clone();
    calc_inc.recalc_all(&mut incremental);
    assert!(calc_inc.graph_is_current());

    let mut fresh = wb.clone();
    let mut calc_full = Calc::new();
    calc_full.recalc_all(&mut fresh);
    assert_eq!(incremental, fresh);
    assert_eq!(calc_inc.graph.len(), calc_full.graph.len());
}

#[test]
fn invalidate_graph_forces_rebuild_on_ensure() {
    let mut wb = chain(50);
    let mut calc = Calc::new();
    calc.recalc_all(&mut wb);
    assert!(calc.graph_is_current());
    calc.invalidate_graph();
    assert!(!calc.graph_is_current());
    // Structural-style rewrite: every formula text changes, old nodes would be wrong.
    for r in 0..50 {
        wb.sheet_mut(0).unwrap().cells.set(CellRef::new(r, 1), Cell::formula(Formula::new("=A1+2")));
    }
    calc.ensure_graph(&wb);
    assert!(calc.graph_is_current());
    calc.recalc_all(&mut wb);
    assert_eq!(wb.sheet(0).unwrap().value(c("B1")), Value::Number(9.0));
}

#[test]
fn recalc_on_load_recomputes_empty_text_formula_cache() {
    let mut wb = Workbook::new();
    wb.sheet_mut(0).unwrap().cells.set(c("A1"), Cell::value(Value::Number(11002048.0)));
    wb.sheet_mut(0).unwrap().cells.set(
        c("B1"),
        Cell {
            value: Value::text(""),
            formula: Some(Arc::new(Formula::new("=A1+1"))),
            style: Default::default(),
        },
    );
    let mut calc = Calc::new();
    calc.recalc_on_load(&mut wb);
    assert_eq!(wb.sheet(0).unwrap().value(c("B1")), Value::Number(11002049.0));
}

#[test]
fn update_nodes_while_invalid_is_a_noop() {
    let mut wb = chain(10);
    let mut calc = Calc::new();
    calc.recalc_all(&mut wb);
    let before = calc.graph.len();
    calc.invalidate_graph();
    wb.sheet_mut(0).unwrap().cells.set(c("B1"), Cell::formula(Formula::new("=A1+99")));
    calc.update_nodes(&mut wb, &[(0, c("B1"))]);
    assert!(!calc.graph_is_current());
    assert_eq!(calc.graph.len(), before, "stale nodes untouched until rebuild");
    calc.ensure_graph(&wb);
    assert_eq!(calc.graph.len(), 10);
}

fn table_col(name: &str) -> gridcraft_model::TableColumn {
    gridcraft_model::TableColumn {
        name: name.into(),
        totals: Default::default(),
        totals_label: None,
        formula: None,
        unique_name: None,
        xml: None,
    }
}

fn push_table(t: &mut T, name: &str, range: &str, cols: &[&str], header: bool, totals: bool) {
    let sh = t.wb.sheet_mut(0).unwrap();
    let id = (sh.tables.len() + 1) as u32;
    sh.tables.push(gridcraft_model::Table {
        id,
        name: name.into(),
        range: gridcraft_core::RangeRef::parse(range).unwrap(),
        header_row: header,
        totals_row: totals,
        columns: cols.iter().map(|c| table_col(c)).collect(),
        style: String::new(),
        banded_rows: false,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: false,
        filter: vec![],
        table_type: None,
        connection_id: None,
    });
}

/// Structured refs resolve to concrete areas at graph time (ThisRow → this cell's row; named
/// column → data body), so they are not stamped `dynamic`.
#[test]
fn structured_ref_precedents_are_static_when_resolvable() {
    use crate::eval::precedents;
    use gridcraft_formula::parse;

    let mut t = T::new();
    t.set("A1", "Referencia");
    t.set("B1", "Qty");
    t.set("A2", "x");
    t.set("B2", "2");
    t.set("A3", "y");
    t.set("B3", "5");
    push_table(&mut t, "Sales", "A1:B3", &["Referencia", "Qty"], true, false);

    let (areas, dynamic) = precedents(&t.wb, 0, c("C2"), &parse("=Sales[@Referencia]").unwrap());
    assert!(!dynamic, "ThisRow must not mark the node dynamic");
    assert_eq!(areas.len(), 1);
    assert_eq!(areas[0].range.a1(), "A2");

    let (areas, dynamic) = precedents(&t.wb, 0, c("D1"), &parse("=SUM(Sales[Qty])").unwrap());
    assert!(!dynamic);
    assert_eq!(areas.len(), 1);
    assert_eq!(areas[0].range.a1(), "B2:B3");

    let (areas, dynamic) = precedents(&t.wb, 0, c("E1"), &parse("=Sales[[#Headers],[Qty]]").unwrap());
    assert!(!dynamic);
    assert_eq!(areas[0].range.a1(), "B1");

    let (_, dynamic) = precedents(&t.wb, 0, c("F1"), &parse("=Missing[Qty]").unwrap());
    assert!(dynamic, "unresolvable table stays dynamic");
}

/// Clearing / editing cells in a table column dirties only formulas that depend on those
/// cells (ThisRow → that row), not every structured-ref formula in the book.
#[test]
fn table_column_edit_does_not_dirty_all_structured_formulas() {
    let rows = 80u32;
    let mut t = T::new();
    t.set("A1", "Referencia");
    t.set("B1", "Out");
    for r in 2..=rows + 1 {
        t.set(&format!("A{r}"), &format!("R{r}"));
        t.set(&format!("B{r}"), ""); // placeholder; formulas after the table exists
    }
    push_table(&mut t, "RefTab", &format!("A1:B{}", rows + 1), &["Referencia", "Out"], true, false);
    for r in 2..=rows + 1 {
        // ThisRow consumer per data row (universal table-column pattern).
        t.set(&format!("B{r}"), &format!("=IF(RefTab[@Referencia]=\"\",\"\",RefTab[@Referencia]&\"!\")"));
    }
    // Unrelated formula: must not be pulled into every table edit via graph.dynamic.
    t.set("Z1", "=99+1");
    t.set("Z2", "=Z1*2");
    assert_eq!(t.num("Z2"), 200.0);
    assert_eq!(t.get("B2"), Value::from("R2!"));

    let hooks = Arc::new(CalcHooks::new());
    let max_total = Arc::new(Mutex::new(0usize));
    let collect = max_total.clone();
    hooks.set_progress(Box::new(move |_done, total| {
        if let Ok(mut v) = collect.lock() {
            *v = (*v).max(total);
        }
    }));
    t.calc.set_hooks(hooks);

    // Clear ~13 Referencia cells — before the fix every structured formula was dynamic, so the
    // cascade (and progress denominator) covered the whole book (~80+). After: only dependents
    // of those cells (ThisRow Out formulas on the same rows) plus the seeds themselves.
    let seeds: Vec<_> = (2..=14).map(|r| (0, c(&format!("A{r}")))).collect();
    for r in 2..=14 {
        t.wb.sheet_mut(0).unwrap().cells.set(c(&format!("A{r}")), Cell::value(Value::Empty));
    }
    assert!(!t.calc.cascade_exceeds(&seeds, 40), "cascade must stay near the cleared rows, not the whole book");
    t.calc.cells_changed(&mut t.wb, &seeds);

    let reported = *max_total.lock().unwrap();
    assert!(
        reported > 0 && reported <= 40,
        "incremental progress denominator should be |dirty| (~cleared rows), got {reported}"
    );
    assert_eq!(t.get("B2"), Value::from(""), "cleared ThisRow ref updates that row");
    assert_eq!(t.get("B15"), Value::from("R15!"), "other rows stay computed (and were not force-dirtied)");
    assert_eq!(t.num("Z2"), 200.0, "unrelated formulas are not in the cascade");
}

/// Whole-column structured refs still depend on the full data body; editing one cell dirties
/// every formula that reads that column (Excel-like), but not unrelated sheets of ThisRow-only deps.
#[test]
fn whole_column_structured_ref_dirties_column_readers_only() {
    let mut t = T::new();
    t.set("A1", "Qty");
    t.set("A2", "1");
    t.set("A3", "2");
    t.set("A4", "3");
    push_table(&mut t, "Q", "A1:A4", &["Qty"], true, false);
    t.set("C1", "=SUM(Q[Qty])");
    t.set("C2", "=SUM(Q[Qty])*10");
    t.set("D1", "=100");
    t.set("D2", "=D1+1");
    assert_eq!((t.num("C1"), t.num("C2")), (6.0, 60.0));

    let seeds = [(0, c("A3"))];
    assert!(!t.calc.cascade_exceeds(&seeds, 10));
    assert!(t.calc.cascade_exceeds(&seeds, 1), "at least the SUM readers are dirty");
    t.wb.sheet_mut(0).unwrap().cells.set(c("A3"), Cell::value(Value::Number(20.0)));
    t.calc.cells_changed(&mut t.wb, &seeds);
    assert_eq!((t.num("C1"), t.num("C2")), (24.0, 240.0));
    assert_eq!(t.num("D2"), 101.0);
}

/// Whole-column structured precedents must track table geometry after the table grows.
/// Pinning `Sales[Qty]` at graph time and then expanding the table without refreshing those
/// Areas leaves dependents blind to new rows (the post–resolve_struct regression).
#[test]
fn expanding_table_refreshes_whole_column_structured_precedents() {
    use crate::eval::precedents;
    use gridcraft_formula::parse;

    let mut t = T::new();
    t.set("A1", "Qty");
    t.set("A2", "1");
    t.set("A3", "2");
    push_table(&mut t, "Sales", "A1:A3", &["Qty"], true, false);
    t.set("C1", "=SUM(Sales[Qty])");
    assert_eq!(t.num("C1"), 3.0);

    let (areas, _) = precedents(&t.wb, 0, c("C1"), &parse("=SUM(Sales[Qty])").unwrap());
    assert_eq!(areas[0].range.a1(), "A2:A3");

    // Grow the table and add a row the way auto-expand does — without a full rebuild yet.
    t.wb.sheet_mut(0).unwrap().tables[0].range.end.row = 3; // A1:A4
    t.wb.sheet_mut(0).unwrap().cells.set(c("A4"), Cell::value(Value::Number(10.0)));

    // Stale graph: C1 still depends only on A2:A3, so A4 does not dirty it.
    let seeds = [(0, c("A4"))];
    assert!(
        !t.calc.cascade_exceeds(&seeds, 1),
        "before refresh, editing the new row must not dirty SUM(Sales[Qty])"
    );

    // After nodes that read the table are rebuilt against current geometry, the new row is visible.
    t.calc.rebuild(&t.wb);
    let (areas, _) = precedents(&t.wb, 0, c("C1"), &parse("=SUM(Sales[Qty])").unwrap());
    assert_eq!(areas[0].range.a1(), "A2:A4");
    t.calc.cells_changed(&mut t.wb, &seeds);
    assert_eq!(t.num("C1"), 13.0, "after rebuild, A4 must dirty and update SUM(Sales[Qty])");
}

/// Nested LET + ThisRow structured refs still evaluate per row (regression with static precedents).
#[test]
fn nested_let_with_this_row_structured_refs() {
    let mut t = T::new();
    t.set("A1", "Codigo");
    t.set("B1", "Linha");
    t.set("A2", "MB001");
    t.set("A3", "MB002");
    t.set("A4", "MB003");
    let f = concat!(
        "=LET(codigo,LinhaTab[@Codigo],IF(codigo=\"\",0,",
        "LET(achou,LinhaTab[Codigo]=codigo,IF(SUM(--achou)=0,0,MATCH(TRUE,achou,0)))))",
    );
    push_table(&mut t, "LinhaTab", "A1:B4", &["Codigo", "Linha"], true, false);
    t.set("B2", f);
    t.set("B3", f);
    t.set("B4", f);
    assert_eq!(t.num("B2"), 1.0);
    assert_eq!(t.num("B3"), 2.0);
    assert_eq!(t.num("B4"), 3.0);

    t.wb.sheet_mut(0).unwrap().cells.set(c("A3"), Cell::value(Value::Empty));
    t.calc.cells_changed(&mut t.wb, &[(0, c("A3"))]);
    assert_eq!(t.num("B2"), 1.0);
    assert_eq!(t.num("B3"), 0.0, "empty ThisRow codigo → early 0");
    assert_eq!(t.num("B4"), 3.0, "MB003 still at index 3 in the column array");
}

/// Release timing: `cargo test -p gridcraft-calc measure_f9_incremental_graph --release -- --ignored --nocapture`
#[test]
#[ignore = "timing; run with --ignored in release"]
#[allow(clippy::disallowed_methods)]
fn measure_f9_incremental_graph() {
    use std::time::Instant;
    let n = 50_000u32;
    let wb = chain(n);
    let mut calc = Calc::new();
    let t_build = Instant::now();
    calc.rebuild(&wb);
    let rebuild = t_build.elapsed();

    let t_ensure = Instant::now();
    calc.ensure_graph(&wb);
    let ensure = t_ensure.elapsed();

    let mut wb_inc = wb.clone();
    let mut calc_inc = calc.clone();
    let t_f9_inc = Instant::now();
    calc_inc.recalc_all(&mut wb_inc);
    let f9_inc = t_f9_inc.elapsed();

    let mut wb_full = wb.clone();
    let mut calc_full = Calc::new();
    let t_f9_full = Instant::now();
    calc_full.recalc_all(&mut wb_full);
    let f9_full = t_f9_full.elapsed();

    assert_eq!(wb_inc, wb_full);
    eprintln!(
        "F9 graph ({n} formulas): rebuild={rebuild:?}, ensure_when_current={ensure:?}, \
         F9_with_current_graph={f9_inc:?}, F9_fresh_calc_full_rebuild={f9_full:?}"
    );
}

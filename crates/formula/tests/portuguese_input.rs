use gridcraft_formula::{FormulaLocale, parse, parse_input, print, print_input};

fn input(src: &str) -> String {
    print(&parse_input(src, FormulaLocale::Pt, |_| false).unwrap())
}

#[test]
fn portuguese_aliases_and_argument_separators() {
    for (src, expected) in [
        ("=soma(A1:A10)", "SUM(A1:A10)"),
        ("MEDIA(A1:A10)", "AVERAGE(A1:A10)"),
        ("MÉDIA(A1:A10)", "AVERAGE(A1:A10)"),
        ("SE(A1>0;\"Sim\";\"Não\")", "IF(A1>0,\"Sim\",\"Não\")"),
        ("CONT.NUM(A1:A10)", "COUNT(A1:A10)"),
        ("CONT.NÚM(A1:A10)", "COUNT(A1:A10)"),
        ("SOMASE(A1:A10;\">0\")", "SUMIF(A1:A10,\">0\")"),
        ("CONT.SE(A1:A10;\">0\")", "COUNTIF(A1:A10,\">0\")"),
        ("PROCV(A2;D:E;2;FALSO)", "VLOOKUP(A2,D:E,2,FALSE)"),
        ("procv(A2;D:E;2;0)", "VLOOKUP(A2,D:E,2,0)"),
        ("PROCX(A2;D:D;E:E)", "XLOOKUP(A2,D:D,E:E)"),
        ("CORRESP(A2;D:D;0)", "MATCH(A2,D:D,0)"),
        ("INDICE(D:E;2;1)", "INDEX(D:E,2,1)"),
        ("ÍNDICE(D:E;2;1)", "INDEX(D:E,2,1)"),
        ("HOJE()", "TODAY()"),
        ("AGORA()", "NOW()"),
        ("=verdadeiro", "TRUE"),
        ("=falso", "FALSE"),
        ("VERDADEIRO()", "TRUE()"),
        ("FALSO()", "FALSE()"),
        ("SOMA(1;SUM(2;3);MEDIA(4;6))", "SUM(1,SUM(2,3),AVERAGE(4,6))"),
        ("SE(VERDADEIRO;;)", "IF(TRUE,,)"),
        ("LAMBDA(x;x+1)(2)", "LAMBDA(x,x+1)(2)"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
}

#[test]
fn portuguese_display_round_trips() {
    let expr = parse("VLOOKUP(A2,D:E,2,FALSE)").unwrap();
    let display = print_input(&expr, FormulaLocale::Pt, |_| false).unwrap();
    assert_eq!(display, "PROCV(A2;D:E;2;FALSO)");
}

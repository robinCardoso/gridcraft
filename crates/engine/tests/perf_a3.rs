// Local machine-only benchmark (times a real workbook on this desktop); the wasm clock lint
// doesn't apply: this test target never builds in the web app.
#![allow(clippy::disallowed_methods)]

use gridcraft_engine::Session;
use serde_json::json;

#[test]
fn bench_a3_sc() {
    let t0 = std::time::Instant::now();
    let mut s = Session::new();
    let r = s.execute("file.open", json!({"path": "c:\\Users\\Robson-PC\\.antigravity\\projetos\\grif-excel\\06 Lista SPAAL.xlsm"}));
    println!("File open took: {:?}", t0.elapsed());
    if r.is_err() {
        println!("Error opening: {:?}", r);
        return;
    }

    // Generate 1000 codes separated by newlines
    let mut codes = String::new();
    for i in 0..1000 {
        codes.push_str(&format!("1000{:02}\n", i % 50));
    }

    let tp = std::time::Instant::now();
    let r = s.execute("edit.paste", json!({"sheet": 3, "at": "A15", "text": codes}));
    println!("edit.paste 1000 rows took: {:?}", tp.elapsed());
    assert!(r.is_ok(), "{:?}", r);

    let t1 = std::time::Instant::now();
    let r = s.execute("cell.set", json!({"sheet": 3, "cell": "A3", "input": "SC"}));
    println!("cell.set A3='SC' after paste took: {:?}", t1.elapsed());
    assert!(r.is_ok(), "{:?}", r);
}

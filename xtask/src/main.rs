//! Workspace tooling: `cargo xtask <command>` (or `cargo run -p xtask -- <command>`).
//!
//! Pure Rust (std + serde_json, plus the engine for `parity`). External tools (`cargo`, `git`)
//! are invoked through `std::process::Command`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod assets;
mod ico;
mod keytips;
mod layers;
#[cfg_attr(not(feature = "engine"), allow(dead_code))]
mod parity;
mod stats;
mod version;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci              fmt --check, clippy -D warnings, test, assets, layers, wasm, parity --check
                  (stops at the first failure; the parity check runs only if docs/parity-checklist.md exists)
  assets          check that every icon/image/font/sample file is attributed in ATTRIBUTION.md
  layers          enforce the crate dependency layering (AGENTS.md \"Layering\") + the freeze
                  guard: the interface layer never serializes or recalculates inline
  perf [--budget MS] [--rows N]
                  run the command-budget test on a generated corpus (default: 2 000 ms per
                  command, 2 000 rows) and fail on the first command past the budget
                  (docs/plano-responsividade.md, Fase 6)
  wasm            cargo check --target wasm32-unknown-unknown for the L0-L5 crates (+ gridcraft-web)
  parity [--check] [--out PATH]
                  write docs/parity-checklist.md from gridcraft_engine::catalog (implemented vs missing ids)
  keytips [--check]
                  regenerate crates/ui-egui/src/keytips/generated.rs from keytips.json (--check: fail if stale)
  version [set X.Y.Z[-pre]]
                  print or set the workspace version ([workspace.package] in Cargo.toml)
  stats [--exact] count tests and lines per crate (--exact: ask the test harness via `-- --list`)
  ico <out.ico> <in.png>...
                  pack square PNGs (<= 256 px) into a Windows .ico (see packaging/icons.sh)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("assets") => assets::run(&root()),
        Some("layers") => cmd_layers(),
        Some("perf") => cmd_perf(&rest),
        Some("wasm") => cmd_wasm(),
        Some("ci") => cmd_ci(),
        Some("stats") => stats::run(&root(), rest.contains(&"--exact")),
        Some("parity") => parity::run(&root(), &rest),
        Some("keytips") => keytips::run(&root(), &rest),
        Some("ico") => ico::run(&rest),
        Some("version") => version::run(&root(), &rest),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (AGENTS.md \"Layering\")\n");
    println!("{:<28} {:<10} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!("{:<28} {:<10} {}", c.name, layers::describe(layers::classify(&c.name)), ws.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        return Err(format!("{} layering violation(s)", violations.len()));
    }
    freeze_guard(&root())
}

/// Fase 6 of docs/plano-responsividade.md: the interface layer never serializes a workbook,
/// never recalculates the graph and never writes workbook files inline — long work goes through
/// `crates/ui-egui/src/jobs.rs`, and opens through the two documented async paths in `lib.rs`
/// (`open_path_async` / `open_bytes_async`). A grep keeps the rule from eroding. Cheap
/// filesystem metadata (recent files, fonts, desktop theme, screenshots) stays allowed: it is
/// already threaded or per-frame cheap.
fn freeze_guard(root: &Path) -> Result<(), String> {
    let banned = ["save_bytes", "recalc_all", "recalc_sheet", "cells_changed", "write_file", "dbfile::save"];
    let mut hits: Vec<String> = Vec::new();
    // The interface layer and its desktop shell: long engine work goes through `jobs.rs` (or the
    // documented async open paths), never inline here. The CLI is a batch engine client, like the
    // engine itself, so it is not scanned.
    for src in [root.join("crates/ui-egui/src"), root.join("apps/gridcraft/src")] {
        walk_rs(&src, &mut |file, line_no, line| {
            for symbol in &banned {
                if line.contains(symbol) {
                    hits.push(format!("{}:{line_no}: `{symbol}` in the interface layer", file.display()));
                }
            }
            if line.contains("load_document") && !file.ends_with("lib.rs") {
                hits.push(format!("{}:{line_no}: `load_document` outside the documented async open paths", file.display()));
            }
        });
    }
    println!();
    if hits.is_empty() {
        println!("OK: freeze guard — the interface layer does no inline serialization or recalculation.");
        Ok(())
    } else {
        println!("{} freeze-guard violation(s):", hits.len());
        for h in &hits {
            println!("  - {h}");
        }
        Err(format!("{} freeze-guard violation(s)", hits.len()))
    }
}

/// Visits every `.rs` file under `dir` (recursive) with its 1-based lines.
fn walk_rs(dir: &Path, f: &mut dyn FnMut(&std::path::Path, usize, &str)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, f);
        } else if path.extension().is_some_and(|e| e == "rs")
            && let Ok(body) = std::fs::read_to_string(&path)
        {
            for (i, line) in body.lines().enumerate() {
                f(&path, i + 1, line);
            }
        }
    }
}

/// `cargo xtask perf [--budget MS] [--rows N]`: the command-budget test over a generated corpus,
/// failing on the first command past the budget (Fase 6 of docs/plano-responsividade.md).
fn cmd_perf(rest: &[&str]) -> Result<(), String> {
    let mut budget = 2_000u64;
    let mut rows = 2_000u64;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--budget" => {
                budget = rest.get(i + 1).and_then(|v| v.parse().ok()).ok_or("--budget needs a number of milliseconds")?;
                i += 1;
            }
            "--rows" => {
                rows = rest.get(i + 1).and_then(|v| v.parse().ok()).ok_or("--rows needs a number of rows")?;
                i += 1;
            }
            other => return Err(format!("unknown flag `{other}`; use --budget MS --rows N")),
        }
        i += 1;
    }
    let mut c = cargo();
    c.env("GRIDCRAFT_BUDGET_MS", budget.to_string());
    c.env("GRIDCRAFT_BUDGET_ROWS", rows.to_string());
    c.args(["test", "-p", "gridcraft-engine", "--lib", "every_command_stays_within_budget", "--", "--nocapture"]);
    run(c, &format!("cargo test -p gridcraft-engine --lib every_command_stays_within_budget (budget {budget} ms, corpus {rows} rows)"))?;
    println!("\nperf: every command within the {budget} ms budget on the {rows}-row corpus.");
    Ok(())
}

/// Workspace packages that must build for wasm32: every L0–L5 crate, plus the web app once it
/// exists (`apps/gridcraft-web`).
fn wasm_set() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .filter(|c| match layers::classify(&c.name) {
            Some(layers::Class::Layer(l)) => l <= 5 || c.name == "gridcraft-web",
            _ => false,
        })
        .map(|c| c.name)
        .collect())
}

fn cmd_wasm() -> Result<(), String> {
    let set = wasm_set()?;
    if set.is_empty() {
        return Err("no wasm crates found in the workspace".into());
    }
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown"]);
    for p in &set {
        c.args(["-p", p]);
    }
    let r = run(c, &format!("cargo check --target wasm32-unknown-unknown -p {}", set.join(" -p ")));
    println!("\nwasm32-unknown-unknown check ({}): {}", set.join(", "), if r.is_ok() { "ok" } else { "FAIL" });
    if r.is_err() {
        println!("(if the target is missing: rustup target add wasm32-unknown-unknown)");
    }
    r
}

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        (
            "fmt",
            Box::new(|| {
                let mut c = cargo();
                c.args(["fmt", "--all", "--", "--check"]);
                run(c, "cargo fmt --all -- --check")
            }),
        ),
        (
            "clippy",
            Box::new(|| {
                let mut c = cargo();
                c.args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]);
                run(c, "cargo clippy --workspace --all-targets -- -D warnings")
            }),
        ),
        (
            "test",
            Box::new(|| {
                let mut c = cargo();
                c.args(["test", "--workspace"]);
                run(c, "cargo test --workspace")
            }),
        ),
        ("assets", Box::new(|| assets::run(&root()))),
        ("layers", Box::new(cmd_layers)),
        ("wasm", Box::new(cmd_wasm)),
        (
            "parity",
            Box::new(|| {
                if root().join("docs/parity-checklist.md").exists() {
                    parity::run(&root(), &["--check"])
                } else {
                    println!("parity: docs/parity-checklist.md not generated yet; skipped (run `cargo xtask parity`)");
                    Ok(())
                }
            }),
        ),
        ("keytips", Box::new(|| keytips::run(&root(), &["--check"]))),
    ];
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}

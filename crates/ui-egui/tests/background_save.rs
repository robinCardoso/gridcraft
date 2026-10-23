//! Closing a document while a background save is in flight waits for the job, then closes
//! (docs/plano-responsividade.md, Fase 2: "salvar e fechar").

use gridcraft_engine::Session;
use gridcraft_ui_egui::{Services, SheetApp, jobs};
use serde_json::json;

/// A one-cell workbook marked dirty, with the job thresholds at zero so even a small save goes
/// to a background job (deterministic, no heavy fixture needed).
fn app() -> SheetApp {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    let mut app = SheetApp::new(session, Services::default());
    app.job_thresholds = jobs::Thresholds { save_cells: 0, calc_formulas: 0 };
    app
}

/// Polls until a job reports and applies it, giving the worker thread time to run. A clock-free
/// bounded wait: the crate bans reading the clock in tests.
fn wait_for_job(app: &mut SheetApp, ctx: &egui::Context) -> usize {
    for _ in 0..5000 {
        let done = app.jobs.poll(ctx);
        if !done.is_empty() {
            let n = done.len();
            for r in done {
                app.apply_job(r);
            }
            return n;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    0
}

#[test]
fn closing_waits_for_a_background_save_and_then_closes() {
    let mut app = app();
    let dir = std::env::temp_dir().join("gridcraft-bg-save-close");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("book.xlsx").to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&path);
    let uid = app.session.active().expect("a document is open").uid;

    let saved = app.run("file.save", json!({"path": path})).expect("the save command answers");
    assert_eq!(saved["pending"], json!(true), "the save went to a job: {saved}");
    assert!(app.jobs.is_saving(uid), "its job is registered");

    let closed = app.run("file.close", json!({"force": true})).expect("the close command answers");
    assert_eq!(closed["pending"], json!(true), "the close waits for the save: {closed}");
    assert!(app.session.active().is_some(), "the document stays open while the save runs");

    let ctx = egui::Context::default();
    assert_eq!(wait_for_job(&mut app, &ctx), 1, "the save job finished");
    assert!(app.session.active().is_none(), "the document closed once its save finished");
    assert!(std::path::Path::new(&path).exists(), "and the file was written");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_failed_save_keeps_the_document_open() {
    let mut app = app();
    let bad = if cfg!(windows) { "Z:\\no such folder\\book.xlsx" } else { "/no such folder/book.xlsx" };

    let saved = app.run("file.save", json!({"path": bad})).expect("the save command answers");
    assert_eq!(saved["pending"], json!(true), "the save went to a job: {saved}");
    let closed = app.run("file.close", json!({"force": true})).expect("the close command answers");
    assert_eq!(closed["pending"], json!(true), "the close waits for the save: {closed}");

    let ctx = egui::Context::default();
    assert_eq!(wait_for_job(&mut app, &ctx), 1, "the save job finished");
    assert!(app.session.active().is_some(), "a failed save never closes the document");
    assert!(app.message.is_some(), "the save error is reported through the message box");
}

#[test]
fn a_big_export_writes_from_a_job_without_touching_the_document() {
    let mut app = app();
    let dir = std::env::temp_dir().join("gridcraft-bg-export");
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("exported").to_string_lossy().into_owned();
    let target = format!("{base}.csv");
    let _ = std::fs::remove_file(&target);
    let dirty_before = app.session.active().is_some_and(|d| d.is_dirty());

    // The path has no extension: the job appends it, as the inline command does.
    let r = app.run("file.exportCsv", json!({"path": base})).expect("the export command answers");
    assert_eq!(r["pending"], json!(true), "the export went to a job: {r}");
    assert_eq!(r["path"], json!(target), "and it reports the final target");

    let ctx = egui::Context::default();
    assert_eq!(wait_for_job(&mut app, &ctx), 1, "the export job finished");
    assert!(std::path::Path::new(&target).exists(), "the job wrote the file");
    let doc = app.session.active().expect("an export is a copy: the document stays");
    assert_eq!(doc.is_dirty(), dirty_before, "an export leaves the changes flag alone");
    assert_eq!(doc.path, None, "and never becomes the document's file");
    assert!(app.message.is_none(), "nothing to report");
    let _ = std::fs::remove_file(&target);
}

/// Fase 3 (docs/plano-responsividade.md): with the thresholds at zero, a structural edit on the
/// workbook defers its recalculation; the app spawns the calculation job by itself (after the
/// command), and applying it brings the values up to date — the window never blocks.
#[test]
fn a_structural_edit_recalculates_in_the_background() {
    let mut app = app();
    app.run("range.setValues", json!({"range": "A1", "values": [["H"], [1], [2], [3]]})).expect("the data is in");
    app.run("cell.set", json!({"cell": "B1", "input": "=SUM(A2:A4)"})).expect("the formula is in");
    // Thresholds at zero: after_engine already flipped `defer_structural` for this document.
    assert!(app.session.active().is_some_and(|d| d.defer_structural), "the deferral is on for big-workbook mode");

    app.run("home.deleteRows", json!({"rows": "4:4"})).expect("the structural edit answers");
    assert!(app.session.active().is_some_and(|d| d.recalc_pending), "the recalculation is owed");
    assert!(app.jobs.is_calculating(app.session.active().expect("open").uid), "the app spawned the calculation job by itself");

    // Run frames until the job applies (the values converge without any blocking call).
    let ctx = egui::Context::default();
    let mut fresh = false;
    for _ in 0..5000 {
        wait_for_job(&mut app, &ctx);
        if app.session.active().is_some_and(|d| !d.recalc_pending) {
            fresh = true;
            break;
        }
    }
    assert!(fresh, "the background recalculation was applied");
    assert_eq!(app.run("cell.get", json!({"cell": "B1"})).expect("the cell is there")["value"], json!(3.0), "the sum follows the deleted row");
}

/// Fase 3: a huge cascade (6 000 formulas fed by one input cell) defers its recalculation like
/// the structural edits do — the paste itself lands first, the job brings the values, and a later
/// edit of the input cell cancels and requeues on the newest snapshot.
#[test]
fn a_huge_cascade_recalculates_in_the_background() {
    let mut app = app();
    let data: Vec<Vec<serde_json::Value>> = (0..60).map(|r| (0..100).map(|c| json!(r * 100 + c)).collect()).collect();
    app.run("range.setValues", json!({"range": "A1", "values": data})).expect("the grid is filled");
    // 6 000 formulas in one paste (rows 21..=6020 under a header block).
    let lines: String = (0..6000).map(|_| "=SUM(A1:CV20)*2".to_string()).collect::<Vec<_>>().join("\n");
    app.run("edit.paste", json!({"at": "A21", "text": lines})).expect("the paste answers");
    let uid = app.session.active().expect("open").uid;
    let ctx = egui::Context::default();
    // The paste's own cascade (6 000 new formulas) is past the job limit: it defers.
    assert!(app.session.active().is_some_and(|d| d.recalc_pending) || app.jobs.is_calculating(uid), "a 6 000-formula paste goes to the background");
    for _ in 0..5000 {
        wait_for_job(&mut app, &ctx);
        if !app.session.active().is_some_and(|d| d.recalc_pending) {
            break;
        }
    }
    assert!(!app.session.active().is_some_and(|d| d.recalc_pending), "the values converged");
    assert_eq!(app.run("cell.get", json!({"cell": "A21"})).unwrap()["value"], json!(3_998_000.0), "SUM(A1:CV20) over the filled grid, doubled");

    // Editing the input cell: its cascade (6 000 dependents) is past the limit, so the edit
    // defers too — the job cancels, requeues on the newest snapshot and the value converges.
    app.run("cell.set", json!({"cell": "A1", "input": "1000"})).expect("the edit answers");
    for _ in 0..5000 {
        wait_for_job(&mut app, &ctx);
        if !app.session.active().is_some_and(|d| d.recalc_pending) {
            break;
        }
    }
    assert!(!app.session.active().is_some_and(|d| d.recalc_pending), "the requeued recalculation converged");
    assert_eq!(
        app.run("cell.get", json!({"cell": "A21"})).unwrap()["value"],
        json!(4_000_000.0),
        "the edit propagated through the 6 000-formula cascade"
    );
}

/// Regression: `after_engine` while a calc job is owed must NOT cancel a still-current snapshot
/// (autosave / finished-job apply used to restart every worker at 0 %). An edit that replaces the
/// workbook Arc still cancels and requeues.
#[test]
fn after_engine_keeps_current_calc_job_while_pending() {
    let mut app = app();
    let data: Vec<Vec<serde_json::Value>> = (0..60).map(|r| (0..100).map(|c| json!(r * 100 + c)).collect()).collect();
    app.run("range.setValues", json!({"range": "A1", "values": data})).expect("the grid is filled");
    let lines: String = (0..6000).map(|_| "=SUM(A1:CV20)*2".to_string()).collect::<Vec<_>>().join("\n");
    app.run("edit.paste", json!({"at": "A21", "text": lines})).expect("the paste answers");
    let uid = app.session.active().expect("open").uid;
    assert!(app.jobs.is_calculating(uid), "the deferred paste spawned a calc job");
    let snap = app.jobs.calculating_snapshot(uid).expect("snapshot").clone();

    // Simulate what a finished autosave (or any non-edit `after_engine`) used to do: cancel the
    // running job just because `recalc_pending` is still true.
    for _ in 0..5 {
        app.after_engine();
    }
    assert!(app.jobs.is_calculating(uid), "a current-snapshot job survives after_engine");
    assert!(
        app.jobs.calculating_snapshot(uid).is_some_and(|s| std::sync::Arc::ptr_eq(s, &snap)),
        "the same job keeps running (no cancel/restart loop at 0 %)"
    );

    let ctx = egui::Context::default();
    // Progress must leave 0 % (spawn nudges immediately; cooperative reports follow).
    let mut saw_progress = false;
    for _ in 0..5000 {
        let _ = app.jobs.poll(&ctx);
        if app.jobs.status_for(uid).is_some_and(|j| j.progress > 0.0) {
            saw_progress = true;
            break;
        }
        if !app.jobs.is_calculating(uid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(saw_progress, "calc progress advances above 0 %");

    for _ in 0..5000 {
        wait_for_job(&mut app, &ctx);
        if !app.session.active().is_some_and(|d| d.recalc_pending) {
            break;
        }
    }
    assert!(!app.session.active().is_some_and(|d| d.recalc_pending), "the undisturbed job converges");
}

/// User Cancel on a calc job must stop the worker (hooks flag) and clear the owed recalc so
/// `after_engine` does not immediately re-spawn "Calculando…".
#[test]
fn user_cancel_clears_calculating_and_does_not_respawn() {
    let mut app = app();
    let data: Vec<Vec<serde_json::Value>> = (0..60).map(|r| (0..100).map(|c| json!(r * 100 + c)).collect()).collect();
    app.run("range.setValues", json!({"range": "A1", "values": data})).expect("the grid is filled");
    let lines: String = (0..6000).map(|_| "=SUM(A1:CV20)*2".to_string()).collect::<Vec<_>>().join("\n");
    app.run("edit.paste", json!({"at": "A21", "text": lines})).expect("the paste answers");
    let uid = app.session.active().expect("open").uid;
    assert!(app.jobs.is_calculating(uid), "the deferred paste spawned a calc job");
    let job_id = app.jobs.status_for(uid).expect("status").id;

    // Same effect as the status-bar X: cancel the job and drop the owed recalc.
    app.jobs.cancel(job_id);
    if let Some(d) = app.session.active_mut() {
        d.recalc_pending = false;
        d.recalc_cascade = None;
    }

    let ctx = egui::Context::default();
    for _ in 0..5000 {
        for r in app.jobs.poll(&ctx) {
            app.apply_job(r);
        }
        if !app.jobs.is_calculating(uid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.jobs.is_calculating(uid), "the cancelled job finishes");
    for _ in 0..10 {
        app.after_engine();
    }
    assert!(!app.jobs.is_calculating(uid), "Cancel must not re-spawn Calculando…");
    assert!(!app.session.active().is_some_and(|d| d.recalc_pending), "the owed recalc was cleared");
}

/// Deferred paste records cascade seeds so the job evaluates the dirty set, not a blind full F9.
#[test]
fn deferred_paste_records_cascade_seeds() {
    let mut app = app();
    let data: Vec<Vec<serde_json::Value>> = (0..60).map(|r| (0..100).map(|c| json!(r * 100 + c)).collect()).collect();
    app.run("range.setValues", json!({"range": "A1", "values": data})).expect("the grid is filled");
    let lines: String = (0..6000).map(|_| "=SUM(A1:CV20)*2".to_string()).collect::<Vec<_>>().join("\n");
    app.run("edit.paste", json!({"at": "A21", "text": lines})).expect("the paste answers");
    let seeds = app.session.active().and_then(|d| d.recalc_cascade.clone());
    assert!(seeds.as_ref().is_some_and(|s| s.len() >= 6000), "paste seeds the deferred cascade: {seeds:?}");
    // Let the job finish so the harness does not leak a running worker into other tests.
    let ctx = egui::Context::default();
    for _ in 0..5000 {
        wait_for_job(&mut app, &ctx);
        if !app.session.active().is_some_and(|d| d.recalc_pending) {
            break;
        }
    }
}

/// Steps the harness until the running jobs finish (applying their outcomes), returning how many
/// concluded.
fn run_jobs_to_end(app: &mut SheetApp, ctx: &egui::Context) -> usize {
    let mut done = 0;
    for _ in 0..5000 {
        done += wait_for_job(app, ctx);
        if done > 0 && app.jobs.is_empty() {
            break;
        }
    }
    done
}

/// Fase 3: Goal Seek runs on a snapshot as a job and lands as ONE undo step.
#[test]
fn goal_seek_runs_as_a_job_and_undoes_in_one_step() {
    let mut app = app();
    app.run("range.setValues", json!({"range": "A1", "values": [[1], [3]]})).expect("the data is in");
    app.run("cell.set", json!({"cell": "B1", "input": "=A1*2+1"})).expect("the formula is in");
    let r = app.run("data.goalSeek", json!({"set": "B1", "to": 9, "changing": "A1"})).expect("the command answers");
    assert_eq!(r["pending"], json!(true), "the goal seek went to a job: {r}");

    let ctx = egui::Context::default();
    assert_eq!(run_jobs_to_end(&mut app, &ctx), 1, "the analysis job finished");
    assert_eq!(app.run("cell.get", json!({"cell": "A1"})).unwrap()["value"], json!(4.0));
    assert_eq!(app.run("cell.get", json!({"cell": "B1"})).unwrap()["value"], json!(9.0));
    assert!(app.toast.is_some(), "the job reports the solution");

    app.run("edit.undo", json!({})).expect("undo answers");
    assert_eq!(app.run("cell.get", json!({"cell": "A1"})).unwrap()["value"], json!(1.0), "one undo reverts the analysis");
    assert_eq!(app.run("cell.get", json!({"cell": "B1"})).unwrap()["value"], json!(3.0));
}

/// Fase 3: the Solver runs as a job too and opens the Solver Results dialog with its answer.
#[test]
fn solver_runs_as_a_job_and_opens_the_results() {
    let mut app = app();
    app.run("range.setValues", json!({"range": "A1", "values": [[0, 0], [0, 0], [0, 0]]})).expect("the data is in");
    app.run("cell.set", json!({"cell": "C1", "input": "=3*A1+5*A2"})).expect("objective");
    app.run("cell.set", json!({"cell": "C2", "input": "=2*A2"})).expect("constraint 1");
    app.run("cell.set", json!({"cell": "C3", "input": "=3*A1+2*A2"})).expect("constraint 2");
    let lp = json!({"objective": "C1", "goal": "max", "variables": "A1:A2", "constraints": ["A1 <= 4", "C2 <= 12", "C3 <= 18"], "method": "simplex"});
    let r = app.run("data.solver", lp).expect("the command answers");
    assert_eq!(r["pending"], json!(true), "the solver went to a job: {r}");

    let ctx = egui::Context::default();
    assert_eq!(run_jobs_to_end(&mut app, &ctx), 1, "the solver job finished");
    assert_eq!(app.run("cell.get", json!({"cell": "A1"})).unwrap()["value"], json!(2.0));
    assert_eq!(app.run("cell.get", json!({"cell": "A2"})).unwrap()["value"], json!(6.0));
    assert_eq!(app.dialog.as_ref().map(|d| d.name()), Some("solverResults"), "the results dialog opens from the job");
}

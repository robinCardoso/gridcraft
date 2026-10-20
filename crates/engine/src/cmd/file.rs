//! File menu: new, open, save, close, export, properties, options.

use serde_json::{Value as Json, json};

use super::*;
use crate::DocState;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "file.new", "New Workbook", ["File"], Some("Cmd+N"), "{sample?: \"budget\"|\"sales\"|\"grades\"}", always, new_workbook),
        cmd!(noundo "file.open", "Open…", ["File"], Some("Cmd+O"), "{path} | {name, base64}, plus {password} for a password-protected workbook and {csvAsText?: bool} to keep every csv/tsv/txt field as text, with no number or date conversion (xlsx, xlsm, xls import, xlsb and ods data import, csv, tsv, txt, json, SQLite .db: every table is a sheet)", always, open),
        cmd!(noundo "file.save", "Save", ["File"], Some("Cmd+S"), "{path?} (xlsx by default; .csv/.tsv/.json/.html/.pdf by extension)", has_doc, save),
        cmd!(noundo "file.saveAs", "Save As…", ["File"], Some("Cmd+Shift+S"), "{path} (.xlsx, .csv, .tsv, .json, .html, or .pdf: a PDF of the active sheet)", has_doc, save_as),
        cmd!(query "file.saveBytes", "Encode Workbook", [], None, "{format?: xlsx|csv|tsv|json|html} → {base64}", has_doc, save_bytes),
        cmd!(noundo "file.close", "Close", ["File"], Some("Cmd+W"), "{force?: bool}", has_doc, close),
        cmd!(noundo "file.exportCsv", "Export as CSV", ["File", "Export"], None, "{path, sheet?}", has_doc, |s, p| export(s, p, "csv")),
        cmd!(noundo "file.exportHtml", "Save as Web Page", ["File", "Export"], None, "{path}", has_doc, |s, p| export(s, p, "html")),
        cmd!(noundo "file.importXlsxToDb", "Convert Workbook to Database", ["File", "Export"], None, "{path, out?, open?: bool (default true)} streams an .xlsx into a SQLite .db (one table per sheet, row 1 = column names) without loading it in memory → {out, tables: [{sheet, rows, columns}], warnings}", always, import_xlsx_to_db),
        cmd!("file.properties", "Properties", ["File"], None, "{title?, subject?, author?, company?, keywords?, description?}", has_doc, properties),
        cmd!(
            query "file.options",
            "Options",
            ["File"],
            None,
            "{} → {calculation: {mode, iterative, maxIterations, maxChange}} for the active workbook (the interface opens File › Options; change settings with their own commands, e.g. formulas.calculationOptions, view.theme, app.language.set)",
            always,
            options
        ),
        cmd!(noundo "window.activate", "Switch Window", ["Window"], None, "{index}", has_doc, activate_window),
    ]
}

fn new_workbook(s: &mut Session, p: &Json) -> Result<Json> {
    let i = match str_param(p, "sample") {
        Some(name) => {
            let wb = crate::sample::build(name).ok_or_else(|| bad("file.new", format!("unknown sample `{name}`")))?;
            let title = crate::sample::title(name);
            s.add_document(DocState::new(wb, None, title))
        }
        None => s.new_workbook(),
    };
    Ok(json!({"index": i, "title": s.doc()?.display_title()}))
}

fn open(s: &mut Session, p: &Json) -> Result<Json> {
    if let Some(path) = str_param(p, "path") {
        // Already open? Switch to it.
        if let Some(i) = s.documents().iter().position(|d| d.path.as_deref() == Some(path)) {
            s.set_active(i);
            return Ok(json!({"index": i, "alreadyOpen": true}));
        }
    } else if str_param(p, "base64").is_none() {
        s.ui_requests.push(crate::UiRequest::Dialog("open".into(), json!({})));
        return ok();
    }
    let (doc, warnings) = load_document(p, None)?;
    s.install_document(doc, warnings)
}

/// Loads a document from path or base64 without needing a Session, allowing background execution.
pub fn load_document(p: &Json, mut progress: Option<Box<dyn FnMut(&str) + Send>>) -> Result<(DocState, Vec<String>)> {
    let (name, bytes) = if let Some(b) = str_param(p, "base64") {
        if let Some(cb) = &mut progress {
            cb("Decodificando dados da planilha...");
        }
        (str_param(p, "name").unwrap_or("Book.xlsx").to_string(), crate::io::base64_decode(b).ok_or_else(|| bad("file.open", "invalid base64"))?)
    } else if let Some(path) = str_param(p, "path") {
        let is_db = crate::dbfile::is_db_path(path);
        if let Some(cb) = &mut progress {
            if is_db {
                cb("Abrindo banco SQLite...");
            } else {
                cb("Lendo dados do arquivo...");
            }
        }
        (path.to_string(), if is_db { Vec::new() } else { crate::io::read_file(path)? })
    } else {
        return Err(bad("file.open", "missing `path` or `base64`"));
    };
    let encrypted = gridcraft_xlsx::sniff(&bytes) == gridcraft_xlsx::Format::Encrypted;
    let (mut wb, warnings) = if crate::dbfile::is_db_path(&name) {
        let (w, mut warns) = crate::dbfile::open(&name)?;
        if str_param(p, "path").is_some_and(|p| !crate::dbfile::is_db_path(p)) {
            warns.push("Workbook opened via high-performance SQLite database (.db).".into());
        }
        (w, warns)
    } else {
        if let Some(cb) = &mut progress {
            cb("Processando dados e fórmulas...");
        }
        crate::io::open_bytes_with(&name, &bytes, str_param(p, "password"), bool_param(p, "csvAsText").unwrap_or(false))?
    };
    wb.source_dir = str_param(p, "path").and_then(crate::io::source_dir);
    let imported =
        matches!(crate::io::FileKind::from_path(&name), Some(crate::io::FileKind::Ods | crate::io::FileKind::Xlsb | crate::io::FileKind::Xls))
            || matches!(gridcraft_xlsx::sniff(&bytes), gridcraft_xlsx::Format::Ods | gridcraft_xlsx::Format::Xlsb | gridcraft_xlsx::Format::Xls);
    let title = if imported {
        format!("{} (imported).xlsx", std::path::Path::new(&name).file_stem().and_then(|n| n.to_str()).unwrap_or("Book"))
    } else {
        std::path::Path::new(&name).file_name().and_then(|n| n.to_str()).unwrap_or("Book").to_string()
    };
    let path = if crate::dbfile::is_db_path(&name) {
        Some(name.clone())
    } else {
        str_param(p, "path").filter(|_| !imported && !encrypted).map(str::to_string)
    };
    if let Some(cb) = &mut progress {
        cb("Recalculando e preparando visualização...");
    }
    let mut doc = DocState::new(wb, path, title);
    if crate::io::FileKind::from_path(&name).is_none() && gridcraft_xlsx::sniff(&bytes) == gridcraft_xlsx::Format::Xlsx {
        doc.content_kind = Some(crate::io::FileKind::Xlsx);
    }
    Ok((doc, warnings))
}

fn save(s: &mut Session, p: &Json) -> Result<Json> {
    let path = match str_param(p, "path").map(str::to_string).or_else(|| s.active().and_then(|d| d.path.clone())) {
        Some(p) => p,
        None => {
            s.ui_requests.push(crate::UiRequest::Dialog("saveAs".into(), json!({})));
            return ok();
        }
    };
    // A PDF is a print of the sheet (Page Setup applies), not a copy of the workbook: the
    // document keeps its file and its unsaved changes.
    if std::path::Path::new(&path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        return super::print::export_pdf(s, &json!({"path": path}));
    }
    let d = s.doc_mut()?;
    // A database stores values and formulas only (no formatting, charts...), so saving to another
    // path is a copy that leaves the document and its unsaved changes alone, like a PDF export.
    // Saving a document opened from that very database writes it back and clears the changes.
    if crate::dbfile::is_db_path(&path) {
        crate::dbfile::save(&d.wb, &path)?;
        if d.path.as_deref() == Some(path.as_str()) {
            d.saved = d.wb.clone();
        }
        return Ok(json!({"path": path, "database": true}));
    }
    // The document's own file keeps the format its content had when opened; any other path
    // goes by its extension. Relative paths to linked workbooks are rewritten for the new folder.
    let kind = crate::io::FileKind::from_path(&path).or(d.content_kind.filter(|_| d.path.as_deref() == Some(path.as_str())));
    let rebased = crate::io::source_dir(&path).and_then(|f| gridcraft_xlsx::rebase_links(&d.wb, &f));
    let wb = rebased.as_ref().unwrap_or(&d.wb);
    let bytes = match kind {
        Some(kind) => crate::io::save_bytes_as(wb, &path, kind)?,
        None => crate::io::save_bytes(wb, &path)?,
    };
    crate::io::write_file(&path, &bytes)?;
    // Only a full-fidelity format clears the dirty flag and becomes the document's file.
    if matches!(kind, Some(crate::io::FileKind::Xlsx | crate::io::FileKind::Json)) {
        d.saved = d.wb.clone();
        d.path = Some(path.clone());
    }
    Ok(json!({"path": path, "bytes": bytes.len()}))
}

fn save_as(s: &mut Session, p: &Json) -> Result<Json> {
    if str_param(p, "path").is_none() {
        s.ui_requests.push(crate::UiRequest::Dialog("saveAs".into(), json!({})));
        return ok();
    }
    save(s, p)
}

fn save_bytes(s: &mut Session, p: &Json) -> Result<Json> {
    let fmt = str_param(p, "format").unwrap_or("xlsx");
    let d = s.doc()?;
    let bytes = crate::io::save_bytes(&d.wb, &format!("x.{fmt}"))?;
    let name = format!("{}.{fmt}", std::path::Path::new(&d.display_title()).file_stem().and_then(|x| x.to_str()).unwrap_or("Book"));
    Ok(json!({"name": name, "base64": crate::io::base64_encode(&bytes), "bytes": bytes.len()}))
}

fn close(s: &mut Session, p: &Json) -> Result<Json> {
    let force = bool_param(p, "force").unwrap_or(false);
    let d = s.doc()?;
    if d.is_dirty() && !force {
        s.ui_requests.push(crate::UiRequest::Dialog("saveChanges".into(), json!({"title": d.display_title()})));
        return Ok(json!({"closed": false, "dirty": true}));
    }
    let i = s.active_index();
    s.close_document(i);
    Ok(json!({"closed": true}))
}

fn export(s: &mut Session, p: &Json, ext: &str) -> Result<Json> {
    let path = str_param(p, "path").ok_or_else(|| bad("file.export", "missing `path`"))?.to_string();
    let d = s.doc()?;
    let mut wb = (*d.wb).clone();
    if p.get("sheet").is_some() {
        wb.active_sheet = target_sheet(s, p)?;
    }
    let target = if path.ends_with(&format!(".{ext}")) { path.clone() } else { format!("{path}.{ext}") };
    let bytes = crate::io::save_bytes(&wb, &target)?;
    crate::io::write_file(&target, &bytes)?;
    Ok(json!({"path": target, "bytes": bytes.len()}))
}

fn properties(s: &mut Session, p: &Json) -> Result<Json> {
    edit(s, |cx| {
        let pr = &mut cx.wb.props;
        for (k, slot) in [
            ("title", &mut pr.title),
            ("subject", &mut pr.subject),
            ("author", &mut pr.author),
            ("company", &mut pr.company),
            ("keywords", &mut pr.keywords),
            ("description", &mut pr.description),
        ] {
            if let Some(v) = str_param(p, k) {
                *slot = v.to_string();
            }
        }
        Ok(serde_json::to_value(&cx.wb.props).unwrap_or_default())
    })
}

/// File › Options: reports the workbook-level settings and asks the interface to show its Options
/// page. Nothing changes here; every setting has its own command.
fn options(s: &mut Session, _: &Json) -> Result<Json> {
    s.ui_requests.push(crate::UiRequest::Dialog("options".into(), json!({})));
    let calculation = s.active().map(|d| {
        let c = &d.wb.calc;
        let mode = match c.mode {
            gridcraft_model::CalcMode::Automatic => "automatic",
            gridcraft_model::CalcMode::AutomaticExceptTables => "automaticExceptTables",
            gridcraft_model::CalcMode::Manual => "manual",
        };
        json!({"mode": mode, "iterative": c.iterative, "maxIterations": c.max_iterations, "maxChange": c.max_change})
    });
    Ok(json!({"calculation": calculation}))
}

/// File › Export › Convert Workbook to Database: streams an `.xlsx` into a SQLite `.db` (the sheets
/// never need to fit in memory as a workbook) and, unless `open` is false, opens the result.
fn import_xlsx_to_db(s: &mut Session, p: &Json) -> Result<Json> {
    let path = str_param(p, "path").ok_or_else(|| bad("file.importXlsxToDb", "missing `path`"))?.to_string();
    let out = match str_param(p, "out") {
        Some(o) => o.to_string(),
        None => std::path::Path::new(&path).with_extension("db").to_string_lossy().into_owned(),
    };
    if !crate::dbfile::is_db_path(&out) {
        return Err(bad("file.importXlsxToDb", "`out` must end in .db, .sqlite or .sqlite3"));
    }
    if std::path::Path::new(&out) == std::path::Path::new(&path) {
        return Err(bad("file.importXlsxToDb", "`out` must be a different file than `path`"));
    }
    let (tables, warnings) = crate::dbfile::import_xlsx(&path, &out)?;
    let mut result = json!({
        "out": out,
        "tables": tables.iter().map(|(sheet, rows, columns)| json!({"sheet": sheet, "rows": rows, "columns": columns})).collect::<Vec<_>>(),
        "warnings": warnings,
    });
    if bool_param(p, "open").unwrap_or(true) {
        let opened = open(s, &json!({"path": out}))?;
        if let Some(map) = result.as_object_mut() {
            map.insert("opened".into(), opened);
        }
    }
    Ok(result)
}

fn activate_window(s: &mut Session, p: &Json) -> Result<Json> {
    let i = u32_param(p, "index").ok_or_else(|| bad("window.activate", "missing `index`"))? as usize;
    if i >= s.documents().len() {
        return Err(bad("window.activate", "no such window"));
    }
    s.set_active(i);
    ok()
}

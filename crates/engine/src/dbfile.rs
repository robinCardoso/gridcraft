//! SQLite database files (`.db`): the engine side of `gridcraft-db`. Native only; the web build
//! has no file system, so there these report that databases aren't available.

use gridcraft_model::Workbook;

use crate::{EngineError, Result};

/// Whether `path` names a database file (by extension).
pub fn is_db_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "db" | "sqlite" | "sqlite3"))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;

    pub fn open(path: &str) -> Result<(Workbook, Vec<String>)> {
        gridcraft_db::read_workbook(std::path::Path::new(path), gridcraft_db::DEFAULT_MAX_ROWS)
            .map_err(|e| EngineError::Other(format!("We can't open '{path}': {e}")))
    }

    pub fn save(wb: &Workbook, path: &str) -> Result<()> {
        gridcraft_db::write_workbook(wb, std::path::Path::new(path)).map_err(|e| EngineError::Other(format!("{path}: {e}")))
    }

    /// Streams `xlsx` into a new database at `out`; returns one line per table.
    pub fn import_xlsx(xlsx: &str, out: &str) -> Result<(Vec<(String, u64, usize)>, Vec<String>)> {
        let report = gridcraft_db::import_xlsx(std::path::Path::new(xlsx), std::path::Path::new(out), &mut |sheet, rows| {
            log::info!("importing '{sheet}': {rows} rows");
        })
        .map_err(|e| EngineError::Other(format!("We can't convert '{xlsx}': {e}")))?;
        Ok((report.tables.into_iter().map(|t| (t.sheet, t.rows, t.columns)).collect(), report.warnings))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{import_xlsx, open, save};

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;

    fn unavailable<T>() -> Result<T> {
        Err(EngineError::Other("database files (.db) aren't available in the browser".into()))
    }

    pub fn open(_path: &str) -> Result<(Workbook, Vec<String>)> {
        unavailable()
    }

    pub fn save(_wb: &Workbook, _path: &str) -> Result<()> {
        unavailable()
    }

    pub fn import_xlsx(_xlsx: &str, _out: &str) -> Result<(Vec<(String, u64, usize)>, Vec<String>)> {
        unavailable()
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{import_xlsx, open, save};

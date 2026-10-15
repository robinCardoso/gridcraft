//! SQLite database files (`.db`) for GridCraft.
//!
//! Excel is slow for lookup-heavy models (`COUNTIFS`, `VLOOKUP`, `INDEX/MATCH` over big tables).
//! A database keeps the data in indexed tables instead:
//!
//! - [`import_xlsx`] converts an `.xlsx` to a `.db` **streaming** (the worksheet XML is read row by
//!   row, never loaded whole, so a 300 MB sheet needs only a few MB of memory). Every sheet becomes
//!   a table; row 1 gives the column names.
//! - [`read_workbook`] opens a `.db` as a workbook: every table (or view) is a sheet. It reads
//!   files made by [`import_xlsx`] / [`write_workbook`] as well as any ordinary SQLite database.
//! - [`write_workbook`] saves a workbook as a `.db`.
//!
//! Bookkeeping tables start with `_gc_` and are hidden when a database is opened.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod import;
mod workbook_io;

pub use import::{ImportReport, TableReport, import_xlsx};
pub use workbook_io::{DEFAULT_MAX_ROWS, read_workbook, write_workbook};

/// Errors of database import, open and save.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a valid XLSX package: {0}")]
    Package(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DbError>;

/// Quotes an SQL identifier (table or column name).
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Whether `path` names a database file GridCraft opens through this crate.
pub fn is_db_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "db" | "sqlite" | "sqlite3"))
}

#[cfg(test)]
mod tests;

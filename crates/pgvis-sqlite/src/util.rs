//! Shared utilities for the pgvis-sqlite crate.

/// Escape a SQLite identifier (double-quote any embedded double-quotes).
pub(crate) fn escape_ident(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// Internal error type used across the crate for wrapping low-level SQLite errors
/// before converting them into [`pgvis_core::error::Error`].
#[derive(Debug)]
pub(crate) struct SqliteInternalError(pub String);

impl std::fmt::Display for SqliteInternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SqliteInternalError {}

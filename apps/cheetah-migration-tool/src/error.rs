//! Errors returned by the migration tool.

use std::path::PathBuf;

/// Top-level migration tool error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MigrationError {
    /// I/O failure reading the source file.
    #[error("failed to read source {path}: {source}")]
    SourceRead {
        /// Path to the source file.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// CSV parse failure.
    #[error("failed to parse CSV from {path}: {source}")]
    Csv {
        /// Path to the source file.
        path: PathBuf,
        /// CSV error.
        #[source]
        source: csv::Error,
    },

    /// JSON parse failure.
    #[error("failed to parse JSON from {path}: {source}")]
    Json {
        /// Path to the source file.
        path: PathBuf,
        /// JSON error.
        #[source]
        source: serde_json::Error,
    },

    /// Invalid source record.
    #[error("invalid record at row {row}: {message}")]
    InvalidRecord {
        /// One-based row index.
        row: usize,
        /// Human-readable reason.
        message: String,
    },

    /// Storage backend failure.
    #[error("storage error: {0}")]
    Storage(#[from] cheetah_storage_api::StorageError),

    /// Domain validation failure.
    #[error("domain error: {0}")]
    Domain(#[from] cheetah_domain::DomainError),

    /// Signal types validation failure.
    #[error("types error: {0}")]
    Types(cheetah_signal_types::SignalError),

    /// Generic operational error.
    #[error("{0}")]
    Other(String),
}

impl From<cheetah_signal_types::SignalError> for MigrationError {
    fn from(err: cheetah_signal_types::SignalError) -> Self {
        Self::Types(err)
    }
}

impl MigrationError {
    /// Creates a generic operational error.
    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }
}

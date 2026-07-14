//! Storage errors.

use cheetah_domain::DomainError;

/// Errors returned by storage adapters.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// Generic backend error.
    #[error("backend error: {message}")]
    Backend {
        /// Human-readable description.
        message: String,
    },
    /// Migration failed or produced an inconsistent schema.
    #[error("migration error at version {version}: {message}")]
    Migration {
        /// Version that failed.
        version: i64,
        /// Human-readable description.
        message: String,
    },
    /// Invalid storage configuration.
    #[error("config error: {message}")]
    Config {
        /// Human-readable description.
        message: String,
    },
    /// Connection failure.
    #[error("connection error: {message}")]
    Connection {
        /// Human-readable description.
        message: String,
    },
    /// Storage is unavailable.
    #[error("unavailable: {message}")]
    Unavailable {
        /// Human-readable description.
        message: String,
    },
    /// Optimistic concurrency conflict.
    #[error("concurrent modification: expected {expected}, found {found}")]
    ConcurrentModification {
        /// Expected revision.
        expected: u64,
        /// Revision found.
        found: u64,
    },
    /// Invalid argument.
    #[error("invalid argument: {message}")]
    InvalidArgument {
        /// Human-readable description.
        message: String,
    },
    /// Internal error.
    #[error("internal: {message}")]
    Internal {
        /// Human-readable description.
        message: String,
    },
}

impl StorageError {
    /// Creates a backend error.
    pub fn backend(message: impl Into<String>) -> Self {
        Self::Backend {
            message: message.into(),
        }
    }

    /// Creates a migration error.
    pub fn migration(version: i64, message: impl Into<String>) -> Self {
        Self::Migration {
            version,
            message: message.into(),
        }
    }

    /// Creates a config error.
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
        }
    }

    /// Creates a connection error.
    pub fn connection(message: impl Into<String>) -> Self {
        Self::Connection {
            message: message.into(),
        }
    }

    /// Creates an unavailable error.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable {
            message: message.into(),
        }
    }

    /// Creates an invalid argument error.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
        }
    }

    /// Creates an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Creates a concurrency error.
    pub fn concurrent_modification(expected: u64, found: u64) -> Self {
        Self::ConcurrentModification { expected, found }
    }
}

impl From<StorageError> for DomainError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::Backend { message }
            | StorageError::Connection { message }
            | StorageError::Unavailable { message } => Self::Unavailable { message },
            StorageError::Migration { version, message } => Self::Internal {
                message: format!("migration {version}: {message}"),
            },
            StorageError::Config { message } | StorageError::Internal { message } => {
                Self::Internal { message }
            }
            StorageError::ConcurrentModification { expected, found } => {
                Self::ConcurrentModification { expected, found }
            }
            StorageError::InvalidArgument { message } => Self::InvalidArgument { message },
        }
    }
}

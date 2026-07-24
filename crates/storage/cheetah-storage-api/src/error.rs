//! Storage errors.

use cheetah_domain::DomainError;
use cheetah_signal_types::clamp_str;

/// Maximum byte length of a human-readable message carried by a `StorageError`.
const MAX_STORAGE_ERROR_BYTES: usize = 1024;

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

fn clamp_message(message: impl std::fmt::Display) -> String {
    clamp_str(&message.to_string(), MAX_STORAGE_ERROR_BYTES)
}

impl StorageError {
    /// Creates a backend error.
    pub fn backend(message: impl std::fmt::Display) -> Self {
        Self::Backend {
            message: clamp_message(message),
        }
    }

    /// Creates a migration error.
    pub fn migration(version: i64, message: impl std::fmt::Display) -> Self {
        Self::Migration {
            version,
            message: clamp_message(message),
        }
    }

    /// Creates a config error.
    pub fn config(message: impl std::fmt::Display) -> Self {
        Self::Config {
            message: clamp_message(message),
        }
    }

    /// Creates a connection error.
    pub fn connection(message: impl std::fmt::Display) -> Self {
        Self::Connection {
            message: clamp_message(message),
        }
    }

    /// Creates an unavailable error.
    pub fn unavailable(message: impl std::fmt::Display) -> Self {
        Self::Unavailable {
            message: clamp_message(message),
        }
    }

    /// Creates an invalid argument error.
    pub fn invalid_argument(message: impl std::fmt::Display) -> Self {
        Self::InvalidArgument {
            message: clamp_message(message),
        }
    }

    /// Creates an internal error.
    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::Internal {
            message: clamp_message(message),
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
            | StorageError::Unavailable { message } => Self::unavailable(message),
            StorageError::Migration { version, message } => {
                Self::internal(format!("migration {version}: {message}"))
            }
            StorageError::Config { message } | StorageError::Internal { message } => {
                Self::internal(message)
            }
            StorageError::ConcurrentModification { expected, found } => {
                Self::ConcurrentModification { expected, found }
            }
            StorageError::InvalidArgument { message } => Self::invalid_argument(message),
        }
    }
}

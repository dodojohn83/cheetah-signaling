//! Error mapping for the SQLite storage adapter.

use cheetah_domain::DomainError;
use cheetah_signal_types::clamp_str;
use cheetah_storage_api::StorageError;

/// Maximum byte length of a `DomainError` message produced from a `sqlx::Error`.
const MAX_SQLX_ERROR_BYTES: usize = 1024;

fn clamp(message: impl std::fmt::Display) -> String {
    clamp_str(&message.to_string(), MAX_SQLX_ERROR_BYTES)
}

/// Maps a `sqlx::Error` into a `DomainError` for repository operations.
pub(crate) fn sqlx_to_domain(err: sqlx::Error) -> DomainError {
    match err {
        sqlx::Error::Database(db) => {
            let message = clamp(db.message());
            if db.is_unique_violation() || db.is_foreign_key_violation() {
                DomainError::invalid_argument(message)
            } else {
                DomainError::Unavailable { message }
            }
        }
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => {
            DomainError::Unavailable {
                message: clamp(err),
            }
        }
        sqlx::Error::RowNotFound => DomainError::NotFound {
            entity: "row".to_string(),
            id: "unknown".to_string(),
        },
        sqlx::Error::ColumnNotFound(_) | sqlx::Error::TypeNotFound { .. } => {
            DomainError::internal(clamp(err))
        }
        _ => DomainError::internal(clamp(err)),
    }
}

/// Maps a `sqlx::Error` into a `StorageError` for storage lifecycle operations.
pub(crate) fn sqlx_to_storage(err: sqlx::Error) -> StorageError {
    match err {
        sqlx::Error::Database(db) => StorageError::backend(db.message()),
        sqlx::Error::Io(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => StorageError::unavailable(err),
        sqlx::Error::RowNotFound => StorageError::invalid_argument("row not found"),
        sqlx::Error::ColumnNotFound(_) | sqlx::Error::TypeNotFound { .. } => {
            StorageError::internal(err)
        }
        _ => StorageError::internal(err),
    }
}

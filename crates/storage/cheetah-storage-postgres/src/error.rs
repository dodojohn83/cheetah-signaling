//! Error mapping for the PostgreSQL storage adapter.

use cheetah_domain::DomainError;
use cheetah_storage_api::StorageError;

/// Maps a `sqlx::Error` into a `DomainError` for repository operations.
pub(crate) fn sqlx_to_domain(err: sqlx::Error) -> DomainError {
    match err {
        sqlx::Error::Database(db) => {
            let message = db.message().to_string();
            if db.is_unique_violation() || db.is_foreign_key_violation() {
                DomainError::invalid_argument(message)
            } else {
                DomainError::Unavailable { message }
            }
        }
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => {
            DomainError::Unavailable {
                message: err.to_string(),
            }
        }
        sqlx::Error::RowNotFound => DomainError::NotFound {
            entity: "row".to_string(),
            id: "unknown".to_string(),
        },
        sqlx::Error::ColumnNotFound(_) | sqlx::Error::TypeNotFound { .. } => {
            DomainError::internal(err.to_string())
        }
        _ => DomainError::internal(err.to_string()),
    }
}

/// Maps a `sqlx::Error` into a `StorageError` for storage lifecycle operations.
pub(crate) fn sqlx_to_storage(err: sqlx::Error) -> StorageError {
    match err {
        sqlx::Error::Database(db) => StorageError::backend(db.message().to_string()),
        sqlx::Error::Io(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => StorageError::unavailable(err.to_string()),
        sqlx::Error::RowNotFound => StorageError::invalid_argument("row not found"),
        sqlx::Error::ColumnNotFound(_) | sqlx::Error::TypeNotFound { .. } => {
            StorageError::internal(err.to_string())
        }
        _ => StorageError::internal(err.to_string()),
    }
}

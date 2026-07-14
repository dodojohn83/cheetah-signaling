//! Storage migration ports.

use crate::StorageError;

/// Status of a migration.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MigrationStatus {
    /// No migrations have been applied.
    Empty,
    /// All known migrations are applied.
    Current,
    /// Some migrations are missing.
    Behind {
        /// Current applied version.
        current: i64,
        /// Latest known version.
        target: i64,
    },
    /// Applied migrations exist that are not known.
    Diverged {
        /// Latest applied version.
        applied: i64,
        /// Latest known version.
        known: i64,
    },
}

/// Information about the migration state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MigrationInfo {
    /// Last applied version, if any.
    pub last_applied: Option<i64>,
    /// Latest known version.
    pub latest_known: i64,
    /// Computed status.
    pub status: MigrationStatus,
}

impl MigrationInfo {
    /// Creates a new migration info.
    pub fn new(last_applied: Option<i64>, latest_known: i64, status: MigrationStatus) -> Self {
        Self {
            last_applied,
            latest_known,
            status,
        }
    }
}

/// Migration runner for a storage backend.
#[async_trait::async_trait]
pub trait Migration: Send + Sync {
    /// Applies pending migrations.
    async fn run(&self) -> Result<(), StorageError>;

    /// Returns the current migration status.
    async fn status(&self) -> Result<MigrationInfo, StorageError>;

    /// Validates that the database schema is at the expected version.
    async fn validate(&self) -> Result<(), StorageError>;
}

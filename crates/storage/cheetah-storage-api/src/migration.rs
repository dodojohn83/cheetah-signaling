//! Storage migration ports.

use crate::StorageError;
use std::time::SystemTime;

/// Phase of a zero-downtime schema migration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum MigrationPhase {
    /// Legacy full-schema migration without an explicit phase prefix.
    Baseline,
    /// Add new tables/columns without breaking the previous code path.
    Expand,
    /// Add indexes/constraints needed for the new code path.
    Migrate,
    /// Populate new columns in batches. Resumable and exposes progress.
    Backfill,
    /// Switch reads/writes to the new schema.
    Switch,
    /// Remove deprecated tables/columns after all nodes are upgraded.
    Contract,
}

impl MigrationPhase {
    /// Machine-readable phase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            MigrationPhase::Baseline => "baseline",
            MigrationPhase::Expand => "expand",
            MigrationPhase::Migrate => "migrate",
            MigrationPhase::Backfill => "backfill",
            MigrationPhase::Switch => "switch",
            MigrationPhase::Contract => "contract",
        }
    }
}

impl std::str::FromStr for MigrationPhase {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "baseline" => Ok(MigrationPhase::Baseline),
            "expand" => Ok(MigrationPhase::Expand),
            "migrate" => Ok(MigrationPhase::Migrate),
            "backfill" => Ok(MigrationPhase::Backfill),
            "switch" => Ok(MigrationPhase::Switch),
            "contract" => Ok(MigrationPhase::Contract),
            _ => Err(format!("unknown migration phase: {s}")),
        }
    }
}

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

/// Progress of a single backfill job.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BackfillJob {
    /// Migration version being backfilled.
    pub version: i64,
    /// Human-readable description.
    pub description: String,
    /// Rows processed so far.
    pub processed_rows: u64,
    /// Whether the backfill has finished.
    pub finished: bool,
    /// Last update time.
    pub updated_at: SystemTime,
}

impl BackfillJob {
    /// Creates a new backfill job.
    pub fn new(version: i64, description: impl Into<String>) -> Self {
        Self {
            version,
            description: description.into(),
            processed_rows: 0,
            finished: false,
            updated_at: SystemTime::UNIX_EPOCH,
        }
    }
}

/// Progress summary for all backfill jobs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackfillProgress {
    /// Jobs currently tracked.
    pub jobs: Vec<BackfillJob>,
}

impl BackfillProgress {
    /// Creates a new progress summary.
    pub fn new(jobs: Vec<BackfillJob>) -> Self {
        Self { jobs }
    }
}

/// Migration runner for a storage backend.
#[async_trait::async_trait]
pub trait Migration: Send + Sync {
    /// Applies pending startup migrations (expand, migrate, baseline).
    async fn run(&self) -> Result<(), StorageError>;

    /// Applies pending migrations for a specific phase.
    async fn run_phase(&self, phase: MigrationPhase) -> Result<(), StorageError>;

    /// Resumes backfill jobs until finished.
    async fn run_backfills(&self, batch_size: u64) -> Result<(), StorageError>;

    /// Runs a single backfill batch and returns rows processed.
    async fn run_backfill_step(&self, version: i64, batch_size: u64) -> Result<u64, StorageError>;

    /// Returns the current migration status.
    async fn status(&self) -> Result<MigrationInfo, StorageError>;

    /// Validates that the database schema is at the expected version.
    async fn validate(&self) -> Result<(), StorageError>;

    /// Returns the progress of all backfill jobs.
    async fn backfill_progress(&self) -> Result<BackfillProgress, StorageError>;
}

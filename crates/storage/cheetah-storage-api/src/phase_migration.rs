//! Phase-aware migration framework.
//!
//! Migrations are split into the expand / migrate / backfill / switch / contract
//! lifecycle used for zero-downtime schema evolution:
//!
//! 1. **Expand** adds new columns/tables without breaking the old code path.
//! 2. **Migrate** adds indexes/constraints needed for the new code path.
//! 3. **Backfill** populates new columns in batches; it is resumable and exposes
//!    progress via `_cheetah_backfill_jobs`.
//! 4. **Switch** flips the application to use the new schema (e.g. rename,
//!    drop redundant views).
//! 5. **Contract** removes the old columns/tables and is delayed until all nodes
//!    are on the new version.
//!
//! SQL file names must follow `<version>__<phase>_<description>.sql`. Phase is
//! taken from the first underscore-delimited segment of the description. Files
//! with no recognised phase are treated as `Baseline` (legacy full-schema DDL).

use crate::{BackfillJob, MigrationInfo, MigrationPhase, MigrationStatus, StorageError};
use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime};

/// An embedded migration split by phase.
#[derive(Clone, Debug)]
pub struct VersionedMigration {
    /// Numeric version.
    pub version: i64,
    /// Migration phase.
    pub phase: MigrationPhase,
    /// Original description (without the `.sql` extension).
    pub description: &'static str,
    /// SQL source.
    pub sql: &'static str,
    /// Checksum bytes.
    pub checksum: &'static [u8],
}

impl VersionedMigration {
    /// Creates a phase-aware migration from raw embedded metadata.
    pub fn new(
        version: i64,
        description: &'static str,
        sql: &'static str,
        checksum: &'static [u8],
    ) -> Self {
        let phase = phase_from_description(description);
        Self {
            version,
            phase,
            description,
            sql,
            checksum,
        }
    }
}

fn phase_from_description(description: &str) -> MigrationPhase {
    // sqlx normalizes descriptions by stripping the version prefix and
    // replacing '_' with spaces, so the phase token may be separated by a
    // space or an underscore and may have leading separators.
    let token = description
        .trim_start_matches([' ', '_'])
        .split([' ', '_'])
        .next()
        .unwrap_or_default();
    match token {
        "expand" => MigrationPhase::Expand,
        "migrate" => MigrationPhase::Migrate,
        "backfill" => MigrationPhase::Backfill,
        "switch" => MigrationPhase::Switch,
        "contract" => MigrationPhase::Contract,
        _ => MigrationPhase::Baseline,
    }
}

/// Stable ordering of phases within a single version.
pub const fn phase_order(phase: MigrationPhase) -> u8 {
    match phase {
        MigrationPhase::Expand => 0,
        MigrationPhase::Migrate => 1,
        MigrationPhase::Backfill => 2,
        MigrationPhase::Switch => 3,
        MigrationPhase::Contract => 4,
        MigrationPhase::Baseline => 0,
    }
}

/// Plans and applies phase migrations.
#[derive(Clone, Debug)]
pub struct PhaseMigrationPlanner {
    migrations: Vec<VersionedMigration>,
}

impl PhaseMigrationPlanner {
    /// Creates a planner from a list of migrations.
    ///
    /// Panics if more than one backfill migration shares the same version,
    /// because backfill progress is keyed by `(version)`.
    pub fn new(mut migrations: Vec<VersionedMigration>) -> Self {
        migrations.sort_by_key(|m| (m.version, phase_order(m.phase)));

        let mut seen_backfills = std::collections::HashSet::new();
        for m in &migrations {
            if m.phase == MigrationPhase::Backfill && !seen_backfills.insert(m.version) {
                panic!(
                    "duplicate backfill version {}: {}",
                    m.version, m.description
                );
            }
        }

        Self { migrations }
    }

    /// All known migrations in execution order.
    pub fn all(&self) -> &[VersionedMigration] {
        &self.migrations
    }

    /// Returns migrations that should be applied during normal startup:
    /// expand, migrate and baseline (legacy DDL).
    pub fn startup_migrations(&self) -> Vec<VersionedMigration> {
        self.migrations
            .iter()
            .filter(|m| {
                matches!(
                    m.phase,
                    MigrationPhase::Expand | MigrationPhase::Migrate | MigrationPhase::Baseline
                )
            })
            .cloned()
            .collect()
    }

    /// Returns migrations for a specific phase.
    pub fn phase_migrations(&self, phase: MigrationPhase) -> Vec<VersionedMigration> {
        self.migrations
            .iter()
            .filter(|m| m.phase == phase)
            .cloned()
            .collect()
    }

    /// Computes which migrations have not yet been applied.
    pub fn pending<I>(applied: I, known: &[VersionedMigration]) -> Vec<VersionedMigration>
    where
        I: IntoIterator<Item = (i64, MigrationPhase)>,
    {
        let applied: HashSet<_> = applied.into_iter().collect();
        known
            .iter()
            .filter(|m| !applied.contains(&(m.version, m.phase)))
            .cloned()
            .collect()
    }

    /// Verifies that already-applied migrations have not changed since they were recorded.
    pub fn validate_checksums(
        applied: &[AppliedMigration],
        known: &[VersionedMigration],
    ) -> Result<(), StorageError> {
        let known_by_key: std::collections::HashMap<_, _> = known
            .iter()
            .map(|m| ((m.version, m.phase), m.checksum))
            .collect();
        for a in applied {
            if let Some(known_checksum) = known_by_key.get(&(a.version, a.phase))
                && a.checksum.as_slice() != *known_checksum
            {
                return Err(StorageError::migration(
                    a.version,
                    format!(
                        "checksum mismatch for applied migration {} {:?}: file changed",
                        a.version, a.phase
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Highest version among startup-safe phases (baseline, expand, migrate).
    pub fn latest_startup_version(&self) -> i64 {
        self.startup_migrations()
            .iter()
            .map(|m| m.version)
            .max()
            .unwrap_or(0)
    }

    /// Highest version across all phases.
    pub fn latest_version(&self) -> i64 {
        self.migrations.iter().map(|m| m.version).max().unwrap_or(0)
    }
}

/// A migration already recorded in the tracking table.
#[derive(Clone, Debug)]
pub struct AppliedMigration {
    /// Migration version.
    pub version: i64,
    /// Migration phase.
    pub phase: MigrationPhase,
    /// Checksum stored at application time.
    pub checksum: Vec<u8>,
}

/// Backend operations required to execute phase migrations.
#[async_trait::async_trait]
pub trait PhaseMigrationBackend: Send + Sync {
    /// Initialise the migration state tables.
    async fn init_state_tables(&self) -> Result<(), StorageError>;

    /// Returns the set of already applied migrations, including their checksums.
    async fn list_applied(&self) -> Result<Vec<AppliedMigration>, StorageError>;

    /// Records that a migration phase has been applied.
    async fn record_applied(
        &self,
        version: i64,
        phase: MigrationPhase,
        description: &str,
        checksum: &[u8],
    ) -> Result<(), StorageError>;

    /// Executes arbitrary DDL/DML for a migration.
    async fn execute_migration_sql(&self, sql: &str) -> Result<u64, StorageError>;

    /// Executes a migration and records it as applied in a single transaction.
    ///
    /// The default implementation runs the migration SQL and then records the
    /// applied row separately; backends should override this when they can
    /// provide an atomic DDL transaction.
    ///
    /// Because the migration body and the applied-row insert run in one
    /// transaction, migration scripts must not contain statements that cannot be
    /// executed inside a transaction block (e.g. `CREATE INDEX CONCURRENTLY` on
    /// PostgreSQL). Use `CREATE INDEX IF NOT EXISTS` or perform such operations
    /// manually outside the migration runner.
    async fn apply_migration(&self, m: &VersionedMigration) -> Result<(), StorageError> {
        self.execute_migration_sql(m.sql).await?;
        self.record_applied(m.version, m.phase, m.description, m.checksum)
            .await?;
        Ok(())
    }

    /// Loads a backfill job for the given version, or `None`.
    async fn load_backfill_job(&self, version: i64) -> Result<Option<BackfillJob>, StorageError>;

    /// Inserts or updates a backfill job.
    async fn save_backfill_job(&self, job: &BackfillJob) -> Result<(), StorageError>;

    /// Acquires a cross-process lock for migration execution.
    ///
    /// The default implementation is a no-op; backends that support it (e.g.
    /// Postgres via `pg_advisory_lock`) should override this to serialize
    /// multi-node startups and operator-driven phase runs.
    async fn acquire_migration_lock(&self) -> Result<(), StorageError> {
        Ok(())
    }

    /// Releases the lock acquired by `acquire_migration_lock`.
    async fn release_migration_lock(&self) -> Result<(), StorageError> {
        Ok(())
    }
}

/// Safety bounds for operator-driven backfill loops.
const MAX_BACKFILL_BATCHES: u64 = 10_000;
const MAX_BACKFILL_DURATION: Duration = Duration::from_secs(600);

/// Executor that applies a planned set of phase migrations through a backend.
#[derive(Clone, Debug)]
pub struct PhaseMigrationRunner {
    planner: PhaseMigrationPlanner,
}

impl PhaseMigrationRunner {
    /// Creates a runner from a planner.
    pub fn new(planner: PhaseMigrationPlanner) -> Self {
        Self { planner }
    }

    /// All known migrations.
    pub fn all(&self) -> &[VersionedMigration] {
        self.planner.all()
    }

    /// Returns the startup-safe migrations in execution order.
    pub fn startup_migrations(&self) -> Vec<VersionedMigration> {
        self.planner.startup_migrations()
    }

    /// Highest version among startup-safe phases.
    pub fn latest_startup_version(&self) -> i64 {
        self.planner.latest_startup_version()
    }

    /// Highest version across all phases.
    pub fn latest_version(&self) -> i64 {
        self.planner.latest_version()
    }

    /// Computes migration status from the set of applied startup phases.
    pub fn status_info(&self, applied: &[AppliedMigration]) -> MigrationInfo {
        let startup = self.startup_migrations();
        let applied_keys = applied.iter().map(|a| (a.version, a.phase));
        let pending = PhaseMigrationPlanner::pending(applied_keys, &startup);
        let last_applied = applied.iter().map(|a| a.version).max();
        let latest_startup = self.latest_startup_version();

        let status = if !pending.is_empty() {
            MigrationStatus::Behind {
                current: last_applied.unwrap_or(0),
                target: latest_startup,
            }
        } else if latest_startup == 0 {
            MigrationStatus::Current
        } else {
            match last_applied {
                Some(last) if last == latest_startup => MigrationStatus::Current,
                Some(last) if last < latest_startup => MigrationStatus::Behind {
                    current: last,
                    target: latest_startup,
                },
                Some(last) => MigrationStatus::Diverged {
                    applied: last,
                    known: latest_startup,
                },
                None => MigrationStatus::Empty,
            }
        };
        MigrationInfo::new(last_applied, latest_startup, status)
    }

    /// Runs all startup phases (expand, migrate and baseline).
    pub async fn run_startup(
        &self,
        backend: &dyn PhaseMigrationBackend,
    ) -> Result<(), StorageError> {
        backend.init_state_tables().await?;
        backend.acquire_migration_lock().await?;
        let result = self.apply_locked(backend, |p| p.startup_migrations()).await;
        let _ = backend.release_migration_lock().await;
        result
    }

    /// Runs all pending migrations for a specific phase.
    pub async fn run_phase(
        &self,
        backend: &dyn PhaseMigrationBackend,
        phase: MigrationPhase,
    ) -> Result<(), StorageError> {
        backend.init_state_tables().await?;
        backend.acquire_migration_lock().await?;
        let result = self
            .apply_locked(backend, |p| p.phase_migrations(phase))
            .await;
        let _ = backend.release_migration_lock().await;
        result
    }

    async fn apply_locked<F>(
        &self,
        backend: &dyn PhaseMigrationBackend,
        select: F,
    ) -> Result<(), StorageError>
    where
        F: FnOnce(&PhaseMigrationPlanner) -> Vec<VersionedMigration>,
    {
        let applied = backend.list_applied().await?;
        let candidates = select(&self.planner);
        PhaseMigrationPlanner::validate_checksums(&applied, &candidates)?;
        let applied_keys = applied.iter().map(|a| (a.version, a.phase));
        let pending = PhaseMigrationPlanner::pending(applied_keys, &candidates);
        self.apply_pending(backend, &pending).await
    }

    /// Resumes backfill jobs until all backfill migrations report no remaining rows.
    ///
    /// Each backfill is bounded by `MAX_BACKFILL_BATCHES` and `MAX_BACKFILL_DURATION`
    /// to prevent runaway scripts from blocking the operator call indefinitely.
    pub async fn run_backfills(
        &self,
        backend: &dyn PhaseMigrationBackend,
        batch_size: u64,
    ) -> Result<(), StorageError> {
        backend.init_state_tables().await?;
        let backfills = self.planner.phase_migrations(MigrationPhase::Backfill);
        for m in backfills {
            let start = Instant::now();
            for batch in 0..MAX_BACKFILL_BATCHES {
                if start.elapsed() > MAX_BACKFILL_DURATION {
                    return Err(StorageError::migration(
                        m.version,
                        "backfill exceeded maximum duration",
                    ));
                }
                let rows = self
                    .run_backfill_step(backend, m.version, batch_size)
                    .await?;
                if rows == 0 {
                    break;
                }
                if batch == MAX_BACKFILL_BATCHES - 1 {
                    return Err(StorageError::migration(
                        m.version,
                        "backfill exceeded maximum batch count",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Executes a single backfill batch and returns the number of rows modified.
    pub async fn run_backfill_step(
        &self,
        backend: &dyn PhaseMigrationBackend,
        version: i64,
        batch_size: u64,
    ) -> Result<u64, StorageError> {
        backend.init_state_tables().await?;
        let candidates: Vec<_> = self
            .planner
            .phase_migrations(MigrationPhase::Backfill)
            .into_iter()
            .filter(|m| m.version == version)
            .collect();
        let Some(m) = candidates.first() else {
            return Err(StorageError::migration(
                version,
                "no backfill migration found",
            ));
        };

        let mut job = backend
            .load_backfill_job(version)
            .await?
            .unwrap_or_else(|| BackfillJob::new(version, m.description));
        if job.finished {
            return Ok(0);
        }

        // Substitute a batch size placeholder into the SQL. Backfill scripts are
        // expected to contain `/*BATCH_SIZE*/` and to be idempotent.
        let sql = m.sql.replace("/*BATCH_SIZE*/", &batch_size.to_string());
        let rows = backend.execute_migration_sql(&sql).await?;

        if rows == 0 {
            job.finished = true;
            job.updated_at = SystemTime::now();
            backend
                .record_applied(
                    m.version,
                    MigrationPhase::Backfill,
                    m.description,
                    m.checksum,
                )
                .await?;
        } else {
            job.processed_rows += rows;
            job.updated_at = SystemTime::now();
        }
        backend.save_backfill_job(&job).await?;
        Ok(rows)
    }

    async fn apply_pending(
        &self,
        backend: &dyn PhaseMigrationBackend,
        pending: &[VersionedMigration],
    ) -> Result<(), StorageError> {
        for m in pending {
            backend.apply_migration(m).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm(version: i64, description: &'static str) -> VersionedMigration {
        VersionedMigration::new(version, description, "", &[])
    }

    #[test]
    fn phase_from_description_parsing() {
        // sqlx normalizes descriptions to spaces and may leave a leading space.
        assert_eq!(vm(1, " initial").phase, MigrationPhase::Baseline);
        assert_eq!(vm(2, " expand add status").phase, MigrationPhase::Expand);
        assert_eq!(vm(3, " migrate add index").phase, MigrationPhase::Migrate);
        assert_eq!(vm(4, " backfill status").phase, MigrationPhase::Backfill);
        assert_eq!(vm(5, " switch use status").phase, MigrationPhase::Switch);
        assert_eq!(vm(6, " contract drop old").phase, MigrationPhase::Contract);
        // Underscore-separated descriptions are also accepted for tests/fixtures.
        assert_eq!(vm(2, "expand_add_status").phase, MigrationPhase::Expand);
    }

    #[test]
    fn startup_migrations_skip_backfill_switch_contract() {
        let planner = PhaseMigrationPlanner::new(vec![
            vm(1, " initial"),
            vm(2, " expand add status"),
            vm(2, " migrate add index"),
            vm(2, " backfill status"),
            vm(2, " switch use status"),
            vm(2, " contract drop old"),
        ]);
        let startup = planner.startup_migrations();
        assert_eq!(startup.len(), 3);
        assert!(startup.iter().all(|m| !matches!(
            m.phase,
            MigrationPhase::Backfill | MigrationPhase::Switch | MigrationPhase::Contract
        )));
    }

    #[test]
    fn pending_filters_applied() {
        let known = vec![
            vm(1, " initial"),
            vm(2, " expand add status"),
            vm(2, " migrate add index"),
            vm(2, " backfill status"),
        ];
        let applied = [(1, MigrationPhase::Baseline), (2, MigrationPhase::Expand)];
        let pending = PhaseMigrationPlanner::pending(applied, &known);
        assert_eq!(pending.len(), 2);
        assert!(
            pending
                .iter()
                .any(|m| m.version == 2 && m.phase == MigrationPhase::Migrate)
        );
        assert!(
            pending
                .iter()
                .any(|m| m.version == 2 && m.phase == MigrationPhase::Backfill)
        );
    }
}

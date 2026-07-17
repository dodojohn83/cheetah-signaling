//! PostgreSQL migration runner with expand / migrate / backfill / switch / contract phases.

use cheetah_storage_api::{
    BackfillJob, BackfillProgress, Migration, MigrationInfo, MigrationPhase, MigrationStatus,
    PhaseMigrationBackend, PhaseMigrationPlanner, PhaseMigrationRunner, StorageError,
    VersionedMigration,
};
use sqlx::types::time::OffsetDateTime;
use sqlx::{PgPool, migrate::Migration as SqlxMigration};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations/postgres");

/// PostgreSQL migration runner.
#[derive(Debug, Clone)]
pub struct PostgresMigration {
    pool: PgPool,
    runner: PhaseMigrationRunner,
}

impl PostgresMigration {
    /// Creates a new migration runner for the given pool.
    pub fn new(pool: PgPool) -> Self {
        let migrations: Vec<VersionedMigration> = MIGRATOR
            .iter()
            .map(|m: &SqlxMigration| {
                VersionedMigration::new(
                    m.version,
                    m.description.as_ref(),
                    m.sql.as_ref(),
                    m.checksum.as_ref(),
                )
            })
            .collect();
        let planner = PhaseMigrationPlanner::new(migrations);
        Self {
            pool,
            runner: PhaseMigrationRunner::new(planner),
        }
    }

    fn latest_known(&self) -> i64 {
        self.runner
            .all()
            .iter()
            .map(|m| m.version)
            .max()
            .unwrap_or(0)
    }

    async fn applied_startup_versions(&self) -> Result<Vec<(i64, MigrationPhase)>, StorageError> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT version, phase FROM _cheetah_migrations WHERE phase IN ('baseline', 'expand', 'migrate')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        rows.into_iter()
            .map(|(v, p)| {
                p.parse::<MigrationPhase>()
                    .map(|phase| (v, phase))
                    .map_err(|e| StorageError::migration(v, e))
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl PhaseMigrationBackend for PostgresMigration {
    async fn init_state_tables(&self) -> Result<(), StorageError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _cheetah_migrations (
                version BIGINT NOT NULL,
                phase TEXT NOT NULL,
                description TEXT NOT NULL,
                checksum BYTEA NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (version, phase)
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _cheetah_backfill_jobs (
                version BIGINT PRIMARY KEY,
                description TEXT NOT NULL,
                processed_rows BIGINT NOT NULL DEFAULT 0,
                finished BOOLEAN NOT NULL DEFAULT FALSE,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(())
    }

    async fn list_applied(&self) -> Result<Vec<(i64, MigrationPhase)>, StorageError> {
        let rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT version, phase FROM _cheetah_migrations")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::backend(e.to_string()))?;

        rows.into_iter()
            .map(|(v, p)| {
                p.parse::<MigrationPhase>()
                    .map(|phase| (v, phase))
                    .map_err(|e| StorageError::migration(v, e))
            })
            .collect()
    }

    async fn record_applied(
        &self,
        version: i64,
        phase: MigrationPhase,
        description: &str,
        checksum: &[u8],
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO _cheetah_migrations (version, phase, description, checksum, applied_at)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (version, phase) DO UPDATE SET
                 description = EXCLUDED.description,
                 checksum = EXCLUDED.checksum,
                 applied_at = EXCLUDED.applied_at",
        )
        .bind(version)
        .bind(phase.as_str())
        .bind(description)
        .bind(checksum)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::migration(version, e.to_string()))?;
        Ok(())
    }

    async fn execute_migration_sql(&self, sql: &str) -> Result<u64, StorageError> {
        let result = sqlx::raw_sql(sql)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::migration(0, e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn load_backfill_job(&self, version: i64) -> Result<Option<BackfillJob>, StorageError> {
        let row: Option<BackfillJobRow> = sqlx::query_as(
            "SELECT version, description, processed_rows, finished, updated_at
             FROM _cheetah_backfill_jobs WHERE version = $1",
        )
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(row.map(|r| r.into_job()))
    }

    async fn save_backfill_job(&self, job: &BackfillJob) -> Result<(), StorageError> {
        let updated_at = system_time_to_offset(job.updated_at);
        sqlx::query(
            "INSERT INTO _cheetah_backfill_jobs
             (version, description, processed_rows, finished, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (version) DO UPDATE SET
                 description = EXCLUDED.description,
                 processed_rows = EXCLUDED.processed_rows,
                 finished = EXCLUDED.finished,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(job.version)
        .bind(job.description.as_str())
        .bind(job.processed_rows as i64)
        .bind(job.finished)
        .bind(updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Migration for PostgresMigration {
    async fn run(&self) -> Result<(), StorageError> {
        self.runner.run_startup(self).await
    }

    async fn run_phase(&self, phase: MigrationPhase) -> Result<(), StorageError> {
        self.runner.run_phase(self, phase).await
    }

    async fn run_backfills(&self, batch_size: u64) -> Result<(), StorageError> {
        self.runner.run_backfills(self, batch_size).await
    }

    async fn run_backfill_step(&self, version: i64, batch_size: u64) -> Result<u64, StorageError> {
        self.runner
            .run_backfill_step(self, version, batch_size)
            .await
    }

    async fn status(&self) -> Result<MigrationInfo, StorageError> {
        self.init_state_tables().await?;
        let applied = self.applied_startup_versions().await?;
        let last_applied = applied.iter().map(|(v, _)| *v).max();
        let latest_known = self.latest_known();
        let status = match last_applied {
            Some(last) if last == latest_known => MigrationStatus::Current,
            Some(last) if last < latest_known => MigrationStatus::Behind {
                current: last,
                target: latest_known,
            },
            Some(last) => MigrationStatus::Diverged {
                applied: last,
                known: latest_known,
            },
            None if latest_known == 0 => MigrationStatus::Current,
            None => MigrationStatus::Empty,
        };
        Ok(MigrationInfo::new(last_applied, latest_known, status))
    }

    async fn validate(&self) -> Result<(), StorageError> {
        self.init_state_tables().await?;
        let info = self.status().await?;
        if info.status == MigrationStatus::Current {
            Ok(())
        } else {
            Err(StorageError::migration(
                info.latest_known,
                format!("schema not current: {:?}", info.status),
            ))
        }
    }

    async fn backfill_progress(&self) -> Result<BackfillProgress, StorageError> {
        let rows: Vec<BackfillJobRow> = sqlx::query_as(
            "SELECT version, description, processed_rows, finished, updated_at
             FROM _cheetah_backfill_jobs ORDER BY version",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(BackfillProgress::new(
            rows.into_iter().map(|r| r.into_job()).collect(),
        ))
    }
}

#[derive(sqlx::FromRow)]
struct BackfillJobRow {
    version: i64,
    description: String,
    processed_rows: i64,
    finished: bool,
    updated_at: OffsetDateTime,
}

impl BackfillJobRow {
    fn into_job(self) -> BackfillJob {
        let mut job = BackfillJob::new(self.version, self.description);
        job.processed_rows = self.processed_rows as u64;
        job.finished = self.finished;
        job.updated_at = UNIX_EPOCH + Duration::from_secs(self.updated_at.unix_timestamp() as u64);
        job
    }
}

fn system_time_to_offset(t: SystemTime) -> OffsetDateTime {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    OffsetDateTime::from_unix_timestamp(secs).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

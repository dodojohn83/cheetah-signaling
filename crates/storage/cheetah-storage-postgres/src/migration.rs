//! PostgreSQL migration runner with expand / migrate / backfill / switch / contract phases.

use cheetah_storage_api::timestamp::{offset_to_system_time, system_time_to_offset};
use cheetah_storage_api::{
    AppliedMigration, BackfillJob, BackfillProgress, Migration, MigrationInfo, MigrationPhase,
    MigrationStatus, PhaseMigrationBackend, PhaseMigrationPlanner, PhaseMigrationRunner,
    StorageError, VersionedMigration,
};
use sqlx::pool::PoolConnection;
use sqlx::types::time::OffsetDateTime;
use sqlx::{PgPool, Postgres, migrate::Migration as SqlxMigration};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Postgres advisory lock key for migration serialization.
/// Chosen as a stable 64-bit constant derived from "CHEETAH".
const MIGRATION_LOCK_ID: i64 = 0x43484545544148;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations/postgres");

/// Executes a raw, trusted SQL script (which may contain multiple statements
/// or trigger bodies) as a single batch inside the given connection.
fn execute_raw_sql<'c>(
    conn: &'c mut sqlx::PgConnection,
    sql: &'static str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u64, StorageError>> + Send + 'c>> {
    use sqlx::Executor;
    Box::pin(async move {
        conn.execute(sqlx::raw_sql(sql))
            .await
            .map(|res| res.rows_affected())
            .map_err(|e| StorageError::backend(e.to_string()))
    })
}

/// PostgreSQL migration runner.
#[derive(Debug, Clone)]
pub struct PostgresMigration {
    pool: PgPool,
    runner: PhaseMigrationRunner,
    /// Dedicated connection used to hold the migration advisory lock across
    /// `acquire`/`release` calls.
    lock_conn: Arc<Mutex<Option<PoolConnection<Postgres>>>>,
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
            lock_conn: Arc::new(Mutex::new(None)),
        }
    }

    async fn seed_from_sqlx_migrations(&self) -> Result<(), StorageError> {
        let table_exists: (bool,) = sqlx::query_as(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_name = '_sqlx_migrations'
            )",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        if !table_exists.0 {
            return Ok(());
        }

        let cheetah_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _cheetah_migrations")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        if cheetah_count.0 == 0 {
            sqlx::query(
                "INSERT INTO _cheetah_migrations (version, phase, description, checksum, applied_at)
                 SELECT version, 'baseline' AS phase, description, checksum, installed_on
                 FROM _sqlx_migrations
                 WHERE success = true
                 ON CONFLICT (version, phase) DO NOTHING",
            )
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        }
        Ok(())
    }

    async fn applied_startup_versions(&self) -> Result<Vec<AppliedMigration>, StorageError> {
        let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
            "SELECT version, phase, checksum FROM _cheetah_migrations WHERE phase IN ('baseline', 'expand', 'migrate')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        rows.into_iter()
            .map(|(v, p, checksum)| {
                p.parse::<MigrationPhase>()
                    .map(|phase| AppliedMigration {
                        version: v,
                        phase,
                        checksum,
                    })
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

        self.seed_from_sqlx_migrations().await?;
        Ok(())
    }

    async fn list_applied(&self) -> Result<Vec<AppliedMigration>, StorageError> {
        let rows: Vec<(i64, String, Vec<u8>)> =
            sqlx::query_as("SELECT version, phase, checksum FROM _cheetah_migrations")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::backend(e.to_string()))?;

        rows.into_iter()
            .map(|(v, p, checksum)| {
                p.parse::<MigrationPhase>()
                    .map(|phase| AppliedMigration {
                        version: v,
                        phase,
                        checksum,
                    })
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

    async fn apply_migration(&self, m: &VersionedMigration) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        execute_raw_sql(&mut tx, m.sql)
            .await
            .map_err(|e| StorageError::migration(m.version, e.to_string()))?;

        sqlx::query(
            "INSERT INTO _cheetah_migrations \
             (version, phase, description, checksum, applied_at) \
             VALUES ($1, $2, $3, $4, now()) \
             ON CONFLICT (version, phase) DO UPDATE SET \
                 description = EXCLUDED.description, \
                 checksum = EXCLUDED.checksum, \
                 applied_at = EXCLUDED.applied_at",
        )
        .bind(m.version)
        .bind(m.phase.as_str())
        .bind(m.description)
        .bind(m.checksum)
        .execute(&mut *tx)
        .await
        .map_err(|e| StorageError::migration(m.version, e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }

    async fn acquire_migration_lock(&self) -> Result<(), StorageError> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(MIGRATION_LOCK_ID)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        let mut guard = self.lock_conn.lock().await;
        *guard = Some(conn);
        Ok(())
    }

    async fn release_migration_lock(&self) -> Result<(), StorageError> {
        let mut guard = self.lock_conn.lock().await;
        if let Some(mut conn) = guard.take() {
            let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(MIGRATION_LOCK_ID)
                .fetch_one(&mut *conn)
                .await;
        }
        Ok(())
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

        row.map(|r| r.into_job()).transpose()
    }

    async fn save_backfill_job(&self, job: &BackfillJob) -> Result<(), StorageError> {
        let updated_at = system_time_to_offset(job.updated_at)?;
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
        // Ensure the tracking tables exist so status/validate work against
        // databases that were previously migrated by an older binary.
        self.init_state_tables().await?;
        let applied = self.applied_startup_versions().await?;
        Ok(self.runner.status_info(&applied))
    }

    async fn validate(&self) -> Result<(), StorageError> {
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
        self.init_state_tables().await?;
        let rows: Vec<BackfillJobRow> = sqlx::query_as(
            "SELECT version, description, processed_rows, finished, updated_at
             FROM _cheetah_backfill_jobs ORDER BY version",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let jobs: Vec<BackfillJob> = rows
            .into_iter()
            .map(|r| r.into_job())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(BackfillProgress::new(jobs))
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
    fn into_job(self) -> Result<BackfillJob, StorageError> {
        let mut job = BackfillJob::new(self.version, self.description);
        job.processed_rows = self.processed_rows as u64;
        job.finished = self.finished;
        job.updated_at = offset_to_system_time(self.updated_at)?;
        Ok(job)
    }
}

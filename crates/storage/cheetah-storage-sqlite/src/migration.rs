//! SQLite migration runner with expand / migrate / backfill / switch / contract phases.

use cheetah_storage_api::{
    AppliedMigration, BackfillJob, BackfillProgress, Migration, MigrationInfo, MigrationPhase,
    MigrationStatus, PhaseMigrationBackend, PhaseMigrationPlanner, PhaseMigrationRunner,
    StorageError, VersionedMigration,
};
use sqlx::{SqlitePool, migrate::Migration as SqlxMigration};
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations/sqlite");

/// Executes a raw, trusted SQL script (which may contain multiple statements
/// or trigger bodies) as a single batch inside the given connection.
fn execute_raw_sql<'c>(
    conn: &'c mut sqlx::SqliteConnection,
    sql: &'static str,
) -> Pin<Box<dyn std::future::Future<Output = Result<u64, StorageError>> + Send + 'c>> {
    use sqlx::Executor;
    Box::pin(async move {
        conn.execute(sqlx::raw_sql(sql))
            .await
            .map(|res| res.rows_affected())
            .map_err(|e| StorageError::backend(e.to_string()))
    })
}

/// SQLite migration runner.
#[derive(Debug, Clone)]
pub struct SqliteMigration {
    pool: SqlitePool,
    runner: PhaseMigrationRunner,
}

impl SqliteMigration {
    /// Creates a new migration runner for the given pool.
    pub fn new(pool: SqlitePool) -> Self {
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

    async fn seed_from_sqlx_migrations(&self) -> Result<(), StorageError> {
        let sqlx_exists: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();

        if sqlx_exists.is_none() {
            return Ok(());
        }

        let cheetah_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _cheetah_migrations")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        if cheetah_count.0 == 0 {
            sqlx::query(
                "INSERT OR IGNORE INTO _cheetah_migrations (version, phase, description, checksum, applied_at)
                 SELECT version, 'baseline' AS phase, description, checksum, installed_on
                 FROM _sqlx_migrations
                 WHERE success = 1",
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
impl PhaseMigrationBackend for SqliteMigration {
    async fn init_state_tables(&self) -> Result<(), StorageError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _cheetah_migrations (
                version INTEGER NOT NULL,
                phase TEXT NOT NULL,
                description TEXT NOT NULL,
                checksum BLOB NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (version, phase)
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _cheetah_backfill_jobs (
                version INTEGER PRIMARY KEY,
                description TEXT NOT NULL,
                processed_rows INTEGER NOT NULL DEFAULT 0,
                finished INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
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
            "INSERT OR REPLACE INTO _cheetah_migrations (version, phase, description, checksum, applied_at)
             VALUES (?, ?, ?, ?, datetime('now'))",
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
            "INSERT OR REPLACE INTO _cheetah_migrations \
             (version, phase, description, checksum, applied_at) \
             VALUES (?, ?, ?, ?, datetime('now'))",
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

    async fn load_backfill_job(&self, version: i64) -> Result<Option<BackfillJob>, StorageError> {
        let row: Option<BackfillJobRow> = sqlx::query_as(
            "SELECT version, description, processed_rows, finished, updated_at
             FROM _cheetah_backfill_jobs WHERE version = ?",
        )
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(row.map(|r| r.into_job()))
    }

    async fn save_backfill_job(&self, job: &BackfillJob) -> Result<(), StorageError> {
        let updated = humantime_since(job.updated_at);
        sqlx::query(
            "INSERT OR REPLACE INTO _cheetah_backfill_jobs
             (version, description, processed_rows, finished, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(job.version)
        .bind(job.description.as_str())
        .bind(job.processed_rows as i64)
        .bind(job.finished as i64)
        .bind(updated)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Migration for SqliteMigration {
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
    finished: i64,
    updated_at: String,
}

impl BackfillJobRow {
    fn into_job(self) -> BackfillJob {
        let mut job = BackfillJob::new(self.version, self.description);
        job.processed_rows = self.processed_rows as u64;
        job.finished = self.finished != 0;
        job.updated_at = parse_humantime(&self.updated_at).unwrap_or(UNIX_EPOCH);
        job
    }
}

fn humantime_since(t: SystemTime) -> String {
    let nanos = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as i128;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn parse_humantime(s: &str) -> Option<SystemTime> {
    let dt = OffsetDateTime::parse(s, &Rfc3339).ok()?;
    let nanos = dt.unix_timestamp_nanos();
    if nanos < 0 {
        return Some(UNIX_EPOCH);
    }
    Some(UNIX_EPOCH + Duration::from_nanos(nanos as u64))
}

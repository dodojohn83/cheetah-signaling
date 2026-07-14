//! SQLite migration runner.

use cheetah_storage_api::{Migration, MigrationInfo, MigrationStatus, StorageError};
use sqlx::SqlitePool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations/sqlite");

/// SQLite migration runner.
#[derive(Debug, Clone)]
pub struct SqliteMigration {
    pool: SqlitePool,
}

impl SqliteMigration {
    /// Creates a new migration runner for the given pool.
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn latest_known(&self) -> i64 {
        MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
    }
}

#[async_trait::async_trait]
impl Migration for SqliteMigration {
    async fn run(&self) -> Result<(), StorageError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))
    }

    async fn status(&self) -> Result<MigrationInfo, StorageError> {
        let latest_known = self.latest_known();
        let rows: Vec<i64> = sqlx::query_scalar::<sqlx::Sqlite, i64>(
            "SELECT version FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let last_applied = rows.last().copied();
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
}

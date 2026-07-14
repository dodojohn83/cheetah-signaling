//! Operation step repository for SQLite.

use cheetah_storage_api::{OperationStep, OperationStepRepository, StorageError};
use sqlx::FromRow;
use sqlx::SqlitePool;
use time::OffsetDateTime;

#[derive(FromRow)]
struct OperationStepRow {
    #[sqlx(rename = "data")]
    data: sqlx::types::Json<OperationStep>,
}

/// SQLite operation step repository.
#[derive(Debug, Clone)]
pub struct SqliteOperationStepRepository {
    pool: SqlitePool,
}

impl SqliteOperationStepRepository {
    /// Creates a new repository.
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl OperationStepRepository for SqliteOperationStepRepository {
    async fn record(&mut self, step: OperationStep) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO operation_steps (
                tenant_id, operation_id, attempt, owner_epoch, status, error, created_at, data
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(tenant_id, operation_id, attempt) DO UPDATE SET
                owner_epoch = EXCLUDED.owner_epoch,
                status = EXCLUDED.status,
                error = EXCLUDED.error,
                created_at = EXCLUDED.created_at,
                data = EXCLUDED.data",
        )
        .bind(step.tenant_id.as_uuid())
        .bind(step.operation_id.as_uuid())
        .bind(step.attempt as i64)
        .bind(step.owner_epoch as i64)
        .bind(&step.status)
        .bind(step.error.as_deref())
        .bind(OffsetDateTime::now_utc())
        .bind(sqlx::types::Json(&step))
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }

    async fn list(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        operation_id: cheetah_signal_types::OperationId,
    ) -> Result<Vec<OperationStep>, StorageError> {
        let rows: Vec<OperationStepRow> = sqlx::query_as::<sqlx::Sqlite, OperationStepRow>(
            "SELECT data FROM operation_steps WHERE tenant_id = ? AND operation_id = ? ORDER BY attempt",
        )
        .bind(tenant_id.as_uuid())
        .bind(operation_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.data.0).collect())
    }
}

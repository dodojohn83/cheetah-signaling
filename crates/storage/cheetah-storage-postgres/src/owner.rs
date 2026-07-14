//! Owner repository and device owner resolver for PostgreSQL.

use crate::error::sqlx_to_domain;
use cheetah_domain::ports::{DeviceOwnerResolver, OwnerInfo};
use cheetah_storage_api::{OwnerRepository, StorageError};
use sqlx::FromRow;
use sqlx::PgPool;
use time::OffsetDateTime;

#[derive(FromRow)]
struct OwnerRow {
    owner_node_id: uuid::Uuid,
    owner_epoch: i64,
    expires_at: Option<OffsetDateTime>,
}

/// PostgreSQL owner repository.
#[derive(Debug, Clone)]
pub struct PostgresOwnerRepository {
    read_pool: PgPool,
    write_pool: PgPool,
}

impl PostgresOwnerRepository {
    /// Creates a new repository.
    pub const fn new(read_pool: PgPool, write_pool: PgPool) -> Self {
        Self {
            read_pool,
            write_pool,
        }
    }
}

#[async_trait::async_trait]
impl OwnerRepository for PostgresOwnerRepository {
    async fn get(
        &self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
    ) -> Result<Option<OwnerInfo>, StorageError> {
        let row: Option<OwnerRow> = sqlx::query_as::<sqlx::Postgres, OwnerRow>(
            "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = $1 AND device_id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(row.map(|r| OwnerInfo {
            owner_node_id: r.owner_node_id.into(),
            owner_epoch: cheetah_signal_types::OwnerEpoch(r.owner_epoch as u64),
        }))
    }

    async fn set(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
        owner: OwnerInfo,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO device_owners (tenant_id, device_id, owner_node_id, owner_epoch, expires_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT(device_id) DO UPDATE SET
                 tenant_id = EXCLUDED.tenant_id,
                 owner_node_id = EXCLUDED.owner_node_id,
                 owner_epoch = EXCLUDED.owner_epoch,
                 expires_at = EXCLUDED.expires_at,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .bind(owner.owner_node_id.as_uuid())
        .bind(owner.owner_epoch.0 as i64)
        .bind(Option::<OffsetDateTime>::None)
        .bind(OffsetDateTime::now_utc())
        .execute(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }

    async fn clear(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
    ) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM device_owners WHERE tenant_id = $1 AND device_id = $2")
            .bind(tenant_id.as_uuid())
            .bind(device_id.as_uuid())
            .execute(&self.write_pool)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }
}

/// PostgreSQL device owner resolver.
pub struct PostgresDeviceOwnerResolver {
    clock: Box<dyn cheetah_domain::Clock>,
    pool: PgPool,
}

impl PostgresDeviceOwnerResolver {
    /// Creates a new resolver.
    pub fn new(clock: Box<dyn cheetah_domain::Clock>, pool: PgPool) -> Self {
        Self { clock, pool }
    }
}

impl std::fmt::Debug for PostgresDeviceOwnerResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresDeviceOwnerResolver")
            .field("clock", &"<dyn Clock>")
            .field("pool", &self.pool)
            .finish()
    }
}

#[async_trait::async_trait]
impl DeviceOwnerResolver for PostgresDeviceOwnerResolver {
    async fn resolve(
        &self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
    ) -> cheetah_domain::Result<Option<OwnerInfo>> {
        let row: Option<OwnerRow> = sqlx::query_as::<sqlx::Postgres, OwnerRow>(
            "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = $1 AND device_id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_domain)?;

        let now = self.clock.now_wall();
        match row {
            Some(r)
                if r.expires_at
                    .map(|exp| exp > now.as_offset())
                    .unwrap_or(true) =>
            {
                Ok(Some(OwnerInfo {
                    owner_node_id: r.owner_node_id.into(),
                    owner_epoch: cheetah_signal_types::OwnerEpoch(r.owner_epoch as u64),
                }))
            }
            _ => Ok(None),
        }
    }
}

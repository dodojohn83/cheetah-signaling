//! Owner repository and device owner resolver for SQLite.

use crate::error::sqlx_to_domain;
use cheetah_domain::ports::{DeviceOwnerResolver, OwnerInfo};
use cheetah_signal_types::{ListCursor, Page, PageRequest, UtcTimestamp};
use cheetah_storage_api::{OwnedDevice, OwnerRepository, StorageError};
use sqlx::FromRow;
use sqlx::SqlitePool;
use time::OffsetDateTime;

fn to_millis(ts: UtcTimestamp) -> i64 {
    let offset = ts.as_offset();
    offset.unix_timestamp() * 1000 + i64::from(offset.nanosecond()) / 1_000_000
}

#[derive(FromRow)]
struct OwnerRow {
    owner_node_id: uuid::Uuid,
    owner_epoch: i64,
    expires_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
struct OwnedDeviceRow {
    tenant_id: uuid::Uuid,
    device_id: uuid::Uuid,
    owner_node_id: uuid::Uuid,
    owner_epoch: i64,
    expires_at: Option<OffsetDateTime>,
    updated_at: OffsetDateTime,
}

/// SQLite owner repository.
#[derive(Debug, Clone)]
pub struct SqliteOwnerRepository {
    read_pool: SqlitePool,
    write_pool: SqlitePool,
}

impl SqliteOwnerRepository {
    /// Creates a new repository.
    pub const fn new(read_pool: SqlitePool, write_pool: SqlitePool) -> Self {
        Self {
            read_pool,
            write_pool,
        }
    }
}

#[async_trait::async_trait]
impl OwnerRepository for SqliteOwnerRepository {
    async fn get(
        &self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
    ) -> Result<Option<OwnerInfo>, StorageError> {
        let row: Option<OwnerRow> = sqlx::query_as::<sqlx::Sqlite, OwnerRow>(
            "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = ? AND device_id = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(row.map(|r| OwnerInfo {
            owner_node_id: r.owner_node_id.into(),
            owner_epoch: cheetah_signal_types::OwnerEpoch(r.owner_epoch as u64),
            lease_until: r.expires_at.map(UtcTimestamp::from_offset),
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
             VALUES (?, ?, ?, ?, ?, ?)
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
        .bind(owner.lease_until.map(|t| t.as_offset()))
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
        sqlx::query("DELETE FROM device_owners WHERE tenant_id = ? AND device_id = ?")
            .bind(tenant_id.as_uuid())
            .bind(device_id.as_uuid())
            .execute(&self.write_pool)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }

    async fn acquire(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
        node_id: cheetah_signal_types::NodeId,
        now: cheetah_signal_types::UtcTimestamp,
        lease_until: cheetah_signal_types::UtcTimestamp,
    ) -> Result<OwnerInfo, StorageError> {
        let updated_at = now.as_offset();
        sqlx::query(
            "INSERT INTO device_owners (tenant_id, device_id, owner_node_id, owner_epoch, expires_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?)
             ON CONFLICT(device_id) DO UPDATE SET
                 tenant_id = EXCLUDED.tenant_id,
                 owner_node_id = EXCLUDED.owner_node_id,
                 owner_epoch = device_owners.owner_epoch + 1,
                 expires_at = EXCLUDED.expires_at,
                 updated_at = EXCLUDED.updated_at
             WHERE (device_owners.expires_at IS NOT NULL AND device_owners.expires_at <= ?)
                OR device_owners.owner_node_id = ?
                OR NOT EXISTS (
                    SELECT 1 FROM cluster_nodes
                    WHERE node_id = device_owners.owner_node_id
                      AND lease_until > ?
                )",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .bind(node_id.as_uuid())
        .bind(lease_until.as_offset())
        .bind(updated_at)
        .bind(updated_at)
        .bind(node_id.as_uuid())
        .bind(to_millis(now))
        .execute(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let row: Option<OwnerRow> = sqlx::query_as::<sqlx::Sqlite, OwnerRow>(
            "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = ? AND device_id = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .fetch_optional(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        match row {
            Some(r) if r.owner_node_id == node_id.as_uuid() => Ok(OwnerInfo {
                owner_node_id: r.owner_node_id.into(),
                owner_epoch: cheetah_signal_types::OwnerEpoch(r.owner_epoch as u64),
                lease_until: r.expires_at.map(UtcTimestamp::from_offset),
            }),
            _ => Err(StorageError::unavailable(
                "device lease held by another node",
            )),
        }
    }

    async fn renew(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
        node_id: cheetah_signal_types::NodeId,
        lease_until: cheetah_signal_types::UtcTimestamp,
    ) -> Result<Option<OwnerInfo>, StorageError> {
        let now = OffsetDateTime::now_utc();
        let result = sqlx::query(
            "UPDATE device_owners
             SET expires_at = ?, updated_at = ?
             WHERE tenant_id = ? AND device_id = ? AND owner_node_id = ? AND (expires_at IS NULL OR expires_at > ?)",
        )
        .bind(lease_until.as_offset())
        .bind(now)
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .bind(node_id.as_uuid())
        .bind(now)
        .execute(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        let row: Option<OwnerRow> = sqlx::query_as::<sqlx::Sqlite, OwnerRow>(
            "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = ? AND device_id = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .fetch_optional(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(row.map(|r| OwnerInfo {
            owner_node_id: r.owner_node_id.into(),
            owner_epoch: cheetah_signal_types::OwnerEpoch(r.owner_epoch as u64),
            lease_until: r.expires_at.map(UtcTimestamp::from_offset),
        }))
    }

    async fn release(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
        node_id: cheetah_signal_types::NodeId,
        epoch: cheetah_signal_types::OwnerEpoch,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "DELETE FROM device_owners WHERE tenant_id = ? AND device_id = ? AND owner_node_id = ? AND owner_epoch = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .bind(node_id.as_uuid())
        .bind(epoch.0 as i64)
        .execute(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }

    async fn list_by_node(
        &self,
        node_id: cheetah_signal_types::NodeId,
        page: PageRequest,
    ) -> Result<Page<OwnedDevice>, StorageError> {
        let page_size = page.clamped_page_size();
        let cursor = match &page.cursor {
            None => None,
            Some(value) => {
                let cursor = ListCursor::decode(value)
                    .map_err(|e| StorageError::invalid_argument(format!("invalid cursor: {e}")))?;
                Some(
                    cursor.parse().map_err(|e| {
                        StorageError::invalid_argument(format!("invalid cursor: {e}"))
                    })?,
                )
            }
        };

        let rows: Vec<OwnedDeviceRow> = match cursor {
            None => sqlx::query_as::<sqlx::Sqlite, OwnedDeviceRow>(
                "SELECT tenant_id, device_id, owner_node_id, owner_epoch, expires_at, updated_at
                     FROM device_owners
                     WHERE owner_node_id = ?
                     ORDER BY updated_at, device_id
                     LIMIT ?",
            )
            .bind(node_id.as_uuid())
            .bind(i64::from(page_size).saturating_add(1))
            .fetch_all(&self.read_pool)
            .await,
            Some((updated_at, device_id)) => sqlx::query_as::<sqlx::Sqlite, OwnedDeviceRow>(
                "SELECT tenant_id, device_id, owner_node_id, owner_epoch, expires_at, updated_at
                     FROM device_owners
                     WHERE owner_node_id = ?
                       AND (updated_at > ? OR (updated_at = ? AND device_id > ?))
                     ORDER BY updated_at, device_id
                     LIMIT ?",
            )
            .bind(node_id.as_uuid())
            .bind(updated_at.as_offset())
            .bind(updated_at.as_offset())
            .bind(device_id)
            .bind(i64::from(page_size).saturating_add(1))
            .fetch_all(&self.read_pool)
            .await,
        }
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let page_size = page_size as usize;
        let has_more = rows.len() > page_size;
        let next_cursor = if has_more {
            let last = &rows[page_size - 1];
            Some(
                ListCursor::new(UtcTimestamp::from_offset(last.updated_at), last.device_id)
                    .map_err(|e| StorageError::internal(format!("failed to encode cursor: {e}")))?
                    .encode()
                    .map_err(|e| StorageError::internal(format!("failed to encode cursor: {e}")))?,
            )
        } else {
            None
        };

        let items: Vec<OwnedDevice> = rows
            .into_iter()
            .take(page_size)
            .map(|r| OwnedDevice {
                tenant_id: r.tenant_id.into(),
                device_id: r.device_id.into(),
                owner: OwnerInfo {
                    owner_node_id: r.owner_node_id.into(),
                    owner_epoch: cheetah_signal_types::OwnerEpoch(r.owner_epoch as u64),
                    lease_until: r.expires_at.map(UtcTimestamp::from_offset),
                },
            })
            .collect();

        let mut result = Page::new(items);
        if let Some(cursor) = next_cursor {
            result = result.with_next_cursor(cursor);
        }
        Ok(result)
    }
}

/// SQLite device owner resolver.
pub struct SqliteDeviceOwnerResolver {
    clock: Box<dyn cheetah_domain::Clock>,
    pool: SqlitePool,
}

impl SqliteDeviceOwnerResolver {
    /// Creates a new resolver.
    pub fn new(clock: Box<dyn cheetah_domain::Clock>, pool: SqlitePool) -> Self {
        Self { clock, pool }
    }
}

impl std::fmt::Debug for SqliteDeviceOwnerResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteDeviceOwnerResolver")
            .field("clock", &"<dyn Clock>")
            .field("pool", &self.pool)
            .finish()
    }
}

#[async_trait::async_trait]
impl DeviceOwnerResolver for SqliteDeviceOwnerResolver {
    async fn resolve(
        &self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
    ) -> cheetah_domain::Result<Option<OwnerInfo>> {
        let row: Option<OwnerRow> = sqlx::query_as::<sqlx::Sqlite, OwnerRow>(
            "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = ? AND device_id = ?",
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
                    lease_until: r.expires_at.map(UtcTimestamp::from_offset),
                }))
            }
            _ => Ok(None),
        }
    }
}

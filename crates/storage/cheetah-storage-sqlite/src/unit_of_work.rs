//! SQLite unit of work.

use crate::error::sqlx_to_domain;
use cheetah_domain::{
    ChannelRepository, DeviceRepository, DomainError, MediaBindingRepository,
    MediaSessionRepository, OperationRepository, Outbox, OwnerInfo, ProcessedMessageRepository,
    UnitOfWork, WebhookConfigRepository, WebhookDeliveryRepository,
};
use cheetah_signal_types::{DeviceId, NodeId, OwnerEpoch, TenantId, UtcTimestamp};
use sqlx::{FromRow, SqlitePool, Transaction};
use time::OffsetDateTime;

/// Unit of work backed by a SQLite transaction.
pub(crate) struct SqliteUnitOfWork {
    pool: SqlitePool,
    tx: Option<Transaction<'static, sqlx::Sqlite>>,
}

impl SqliteUnitOfWork {
    pub(crate) fn new(pool: SqlitePool, tx: Transaction<'static, sqlx::Sqlite>) -> Self {
        Self { pool, tx: Some(tx) }
    }

    /// Returns the current transaction, lazily beginning a new one if the
    /// previous transaction was committed or rolled back. This allows the same
    /// unit-of-work handle to span pre- and post-I/O phases without holding a
    /// transaction open across external calls.
    pub(crate) async fn tx(
        &mut self,
    ) -> cheetah_domain::Result<&mut Transaction<'static, sqlx::Sqlite>> {
        if self.tx.is_none() {
            let tx = self.pool.begin().await.map_err(sqlx_to_domain)?;
            self.tx = Some(tx);
        }
        self.tx
            .as_mut()
            .ok_or_else(|| DomainError::internal("failed to initialize transaction"))
    }
}

#[async_trait::async_trait]
impl UnitOfWork for SqliteUnitOfWork {
    fn device_repository(&mut self) -> &mut dyn DeviceRepository {
        self
    }

    fn channel_repository(&mut self) -> &mut dyn ChannelRepository {
        self
    }

    fn operation_repository(&mut self) -> &mut dyn OperationRepository {
        self
    }

    fn media_session_repository(&mut self) -> &mut dyn MediaSessionRepository {
        self
    }

    fn media_binding_repository(&mut self) -> &mut dyn MediaBindingRepository {
        self
    }

    fn processed_message_repository(&mut self) -> &mut dyn ProcessedMessageRepository {
        self
    }

    fn webhook_config_repository(&mut self) -> &mut dyn WebhookConfigRepository {
        self
    }

    fn webhook_delivery_repository(&mut self) -> &mut dyn WebhookDeliveryRepository {
        self
    }

    fn outbox(&mut self) -> &mut dyn Outbox {
        self
    }

    async fn acquire_ownership(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        now: UtcTimestamp,
        lease_until: UtcTimestamp,
    ) -> cheetah_domain::Result<Option<(OwnerInfo, Option<OwnerInfo>)>> {
        #[derive(FromRow)]
        struct OwnerRow {
            owner_node_id: uuid::Uuid,
            owner_epoch: i64,
            expires_at: Option<OffsetDateTime>,
        }

        fn to_millis(ts: UtcTimestamp) -> i64 {
            let offset = ts.as_offset();
            offset.unix_timestamp() * 1000 + i64::from(offset.nanosecond()) / 1_000_000
        }

        let conn = self.tx().await?.as_mut();

        let candidate_alive: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM cluster_nodes WHERE node_id = ? AND lease_until > ? AND draining = 0",
        )
        .bind(node_id.as_uuid())
        .bind(to_millis(now))
        .fetch_optional(&mut *conn)
        .await
        .map_err(sqlx_to_domain)?;
        if candidate_alive.is_none() {
            return Err(DomainError::unavailable("candidate node is not eligible"));
        }

        let previous = {
            let row: Option<OwnerRow> = sqlx::query_as::<_, OwnerRow>(
                "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = ? AND device_id = ?",
            )
            .bind(tenant_id.as_uuid())
            .bind(device_id.as_uuid())
            .fetch_optional(&mut *conn)
            .await
            .map_err(sqlx_to_domain)?;
            row.map(|r| OwnerInfo {
                owner_node_id: r.owner_node_id.into(),
                owner_epoch: OwnerEpoch(r.owner_epoch as u64),
                lease_until: r.expires_at.map(UtcTimestamp::from_offset),
            })
        };

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
             WHERE ((device_owners.expires_at IS NOT NULL AND device_owners.expires_at <= ?)
                 OR device_owners.owner_node_id = ?
                 OR NOT EXISTS (
                     SELECT 1 FROM cluster_nodes
                     WHERE node_id = device_owners.owner_node_id AND lease_until > ?
                 ))
               AND EXISTS (
                     SELECT 1 FROM cluster_nodes
                     WHERE node_id = ? AND lease_until > ? AND draining = 0
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
        .bind(node_id.as_uuid())
        .bind(to_millis(now))
        .execute(&mut *conn)
        .await
        .map_err(sqlx_to_domain)?;

        let row: Option<OwnerRow> = sqlx::query_as::<_, OwnerRow>(
            "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = ? AND device_id = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .fetch_optional(&mut *conn)
        .await
        .map_err(sqlx_to_domain)?;

        match row {
            Some(r) if r.owner_node_id == node_id.as_uuid() => Ok(Some((
                OwnerInfo {
                    owner_node_id: r.owner_node_id.into(),
                    owner_epoch: OwnerEpoch(r.owner_epoch as u64),
                    lease_until: r.expires_at.map(UtcTimestamp::from_offset),
                },
                previous,
            ))),
            _ => Ok(None),
        }
    }

    async fn commit(&mut self) -> cheetah_domain::Result<()> {
        let tx = self
            .tx
            .take()
            .ok_or_else(|| DomainError::internal("unit of work already consumed"))?;
        tx.commit().await.map_err(sqlx_to_domain)
    }

    async fn rollback(&mut self) -> cheetah_domain::Result<()> {
        let tx = self
            .tx
            .take()
            .ok_or_else(|| DomainError::internal("unit of work already consumed"))?;
        tx.rollback().await.map_err(sqlx_to_domain)
    }
}

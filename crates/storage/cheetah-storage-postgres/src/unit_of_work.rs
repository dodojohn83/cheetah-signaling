//! PostgreSQL unit of work.

use crate::error::sqlx_to_domain;
use cheetah_domain::{
    ChannelRepository, DeviceRepository, DomainError, MediaBindingRepository,
    MediaSessionRepository, OperationRepository, Outbox, OwnerInfo, ProcessedMessageRepository,
    UnitOfWork, WebhookConfigRepository, WebhookDeliveryRepository,
};
use cheetah_signal_types::{DeviceId, NodeId, OwnerEpoch, TenantId, UtcTimestamp};
use sqlx::{FromRow, Transaction};
use time::OffsetDateTime;

/// Unit of work backed by a PostgreSQL transaction.
pub(crate) struct PostgresUnitOfWork {
    pub(crate) tx: Option<Transaction<'static, sqlx::Postgres>>,
}

impl PostgresUnitOfWork {
    pub(crate) fn new(tx: Transaction<'static, sqlx::Postgres>) -> Self {
        Self { tx: Some(tx) }
    }

    pub(crate) fn tx(
        &mut self,
    ) -> cheetah_domain::Result<&mut Transaction<'static, sqlx::Postgres>> {
        self.tx
            .as_mut()
            .ok_or_else(|| DomainError::internal("unit of work already consumed"))
    }
}

#[async_trait::async_trait]
impl UnitOfWork for PostgresUnitOfWork {
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

        let conn = self.tx()?.as_mut();

        let candidate_alive: Option<(i64,)> = sqlx::query_as(
            "SELECT 1::int8 FROM cluster_nodes WHERE node_id = $1 AND lease_until > $2 AND draining = false",
        )
        .bind(node_id.as_uuid())
        .bind(now.as_offset())
        .fetch_optional(&mut *conn)
        .await
        .map_err(sqlx_to_domain)?;
        if candidate_alive.is_none() {
            return Err(cheetah_domain::DomainError::unavailable(
                "candidate node is not eligible",
            ));
        }

        let previous = {
            let row: Option<OwnerRow> = sqlx::query_as::<_, OwnerRow>(
                "SELECT owner_node_id, owner_epoch, expires_at FROM device_owners WHERE tenant_id = $1 AND device_id = $2",
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
        let row: Option<OwnerRow> = sqlx::query_as::<_, OwnerRow>(
            "INSERT INTO device_owners (tenant_id, device_id, owner_node_id, owner_epoch, expires_at, updated_at)
             VALUES ($1, $2, $3, 1, $4, $5)
             ON CONFLICT(device_id) DO UPDATE SET
                 tenant_id = EXCLUDED.tenant_id,
                 owner_node_id = EXCLUDED.owner_node_id,
                 owner_epoch = device_owners.owner_epoch + 1,
                 expires_at = EXCLUDED.expires_at,
                 updated_at = EXCLUDED.updated_at
             WHERE ((device_owners.expires_at IS NOT NULL AND device_owners.expires_at <= $6)
                 OR device_owners.owner_node_id = $7
                 OR NOT EXISTS (
                     SELECT 1 FROM cluster_nodes
                     WHERE node_id = device_owners.owner_node_id AND lease_until > $8
                 ))
               AND EXISTS (
                     SELECT 1 FROM cluster_nodes
                     WHERE node_id = $9 AND lease_until > $10 AND draining = false
                 )
             RETURNING owner_node_id, owner_epoch, expires_at",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .bind(node_id.as_uuid())
        .bind(lease_until.as_offset())
        .bind(updated_at)
        .bind(updated_at)
        .bind(node_id.as_uuid())
        .bind(updated_at)
        .bind(node_id.as_uuid())
        .bind(updated_at)
        .fetch_optional(&mut *conn)
        .await
        .map_err(sqlx_to_domain)?;

        let owner = match row {
            Some(r) => OwnerInfo {
                owner_node_id: r.owner_node_id.into(),
                owner_epoch: OwnerEpoch(r.owner_epoch as u64),
                lease_until: r.expires_at.map(UtcTimestamp::from_offset),
            },
            None => return Ok(None),
        };

        if owner.owner_node_id == node_id {
            Ok(Some((owner, previous)))
        } else {
            Ok(None)
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

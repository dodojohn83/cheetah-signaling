//! PostgreSQL unit of work.

use crate::error::sqlx_to_domain;
use cheetah_domain::{
    ChannelRepository, DeviceRepository, DomainError, MediaBindingRepository,
    MediaSessionRepository, OperationRepository, Outbox, ProcessedMessageRepository, UnitOfWork,
    WebhookConfigRepository, WebhookDeliveryRepository,
};
use sqlx::{PgPool, Transaction};

/// Unit of work backed by a PostgreSQL transaction.
pub(crate) struct PostgresUnitOfWork {
    pool: PgPool,
    tx: Option<Transaction<'static, sqlx::Postgres>>,
}

impl PostgresUnitOfWork {
    pub(crate) fn new(pool: PgPool, tx: Transaction<'static, sqlx::Postgres>) -> Self {
        Self { pool, tx: Some(tx) }
    }

    /// Returns the current transaction, lazily beginning a new one if the
    /// previous transaction was committed or rolled back. This allows the same
    /// unit-of-work handle to span pre- and post-I/O phases without holding a
    /// transaction open across external calls.
    pub(crate) async fn tx(
        &mut self,
    ) -> cheetah_domain::Result<&mut Transaction<'static, sqlx::Postgres>> {
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

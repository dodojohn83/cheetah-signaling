//! SQLite unit of work.

use crate::error::sqlx_to_domain;
use cheetah_domain::{
    ChannelRepository, DeviceRepository, DomainError, MediaBindingRepository,
    MediaSessionRepository, OperationRepository, Outbox, ProcessedMessageRepository, UnitOfWork,
};
use sqlx::Transaction;

/// Unit of work backed by a SQLite transaction.
pub(crate) struct SqliteUnitOfWork {
    pub(crate) tx: Option<Transaction<'static, sqlx::Sqlite>>,
}

impl SqliteUnitOfWork {
    pub(crate) fn new(tx: Transaction<'static, sqlx::Sqlite>) -> Self {
        Self { tx: Some(tx) }
    }

    pub(crate) fn tx(&mut self) -> cheetah_domain::Result<&mut Transaction<'static, sqlx::Sqlite>> {
        self.tx
            .as_mut()
            .ok_or_else(|| DomainError::internal("unit of work already consumed"))
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

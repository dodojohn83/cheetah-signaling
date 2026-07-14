//! Transactional outbox relay.

use crate::EventService;
use cheetah_domain::Clock;
use cheetah_signal_types::{Result, SignalError};
use cheetah_storage_api::Storage;
use std::sync::Arc;

/// Publishes pending outbox events in batches.
#[derive(Clone)]
pub struct OutboxRelay {
    storage: Arc<dyn Storage>,
    publisher: Arc<dyn cheetah_domain::EventPublisher>,
    clock: Arc<dyn Clock>,
    event_service: EventService,
    batch_size: usize,
}

impl std::fmt::Debug for OutboxRelay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboxRelay")
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

impl OutboxRelay {
    /// Creates a new outbox relay.
    pub fn new(
        storage: Arc<dyn Storage>,
        publisher: Arc<dyn cheetah_domain::EventPublisher>,
        clock: Arc<dyn Clock>,
        batch_size: usize,
    ) -> Self {
        Self {
            storage,
            publisher,
            clock,
            event_service: EventService::new(),
            batch_size,
        }
    }

    /// Reads, publishes, and marks a batch of pending outbox events.
    ///
    /// Pending events are read inside a short transaction, published to the
    /// message bus outside any transaction, and then marked inside a second
    /// short transaction. This avoids holding a database transaction open while
    /// waiting for NATS or another external message broker.
    pub async fn run_once(&self) -> Result<usize> {
        let now = self.clock.now_wall();

        // 1. Read pending events and close the read transaction.
        let entries = {
            let mut uow = self
                .storage
                .begin()
                .await
                .map_err(|e| SignalError::from(cheetah_domain::DomainError::from(e)))?;
            let entries = self
                .event_service
                .read_pending(uow.outbox(), now, self.batch_size)
                .await?;
            uow.commit().await?;
            entries
        };

        // 2. Publish to the external message bus without holding a DB tx.
        let results = self
            .event_service
            .publish_events(self.publisher.as_ref(), &entries)
            .await;

        // 3. Record outcomes in a new transaction.
        let mut uow = self
            .storage
            .begin()
            .await
            .map_err(|e| SignalError::from(cheetah_domain::DomainError::from(e)))?;
        let published = self
            .event_service
            .record_results(uow.outbox(), now, &entries, &results)
            .await?;
        uow.commit().await?;

        Ok(published)
    }
}

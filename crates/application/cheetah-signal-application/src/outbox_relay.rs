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
    /// Returns the number of events successfully published. Events that fail
    /// are retried later according to the configured retry policy.
    pub async fn run_once(&self) -> Result<usize> {
        let mut uow = self
            .storage
            .begin()
            .await
            .map_err(|e| SignalError::from(cheetah_domain::DomainError::from(e)))?;
        let published = self
            .event_service
            .publish_pending(
                uow.outbox(),
                self.publisher.as_ref(),
                self.clock.now_wall(),
                self.batch_size,
            )
            .await?;
        uow.commit().await?;
        Ok(published)
    }
}

//! Event publishing service.

use cheetah_domain::{EventPublisher, Outbox};

/// Publishes pending outbox events.
#[derive(Clone, Debug, Default)]
pub struct EventService;

impl EventService {
    /// Creates a new event service.
    pub fn new() -> Self {
        Self
    }

    /// Publishes up to `limit` pending outbox events.
    pub async fn publish_pending(
        &self,
        outbox: &dyn Outbox,
        publisher: &dyn EventPublisher,
        limit: usize,
    ) -> crate::Result<usize> {
        let entries = outbox.pending(limit).await?;
        for entry in &entries {
            publisher.publish(&entry.event).await?;
            outbox.mark_published(entry.event.event_id).await?;
        }
        Ok(entries.len())
    }
}

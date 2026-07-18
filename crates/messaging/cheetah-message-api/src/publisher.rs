//! Domain [`EventPublisher`] adapter for a [`RawEventBus`].

use crate::{RawEventBus, encode_event, event_subject};
use cheetah_domain::{DomainError, DomainEvent, EventPublisher};
use cheetah_signal_types::Event;

/// Publishes a domain [`Event`] by encoding it and sending it through a [`RawEventBus`].
pub async fn publish_domain_event(
    bus: &dyn RawEventBus,
    event: &Event<DomainEvent>,
) -> Result<(), DomainError> {
    let envelope = encode_event(event).map_err(DomainError::from)?;
    let subject = event_subject(event.tenant_id, "domain_event");
    bus.publish(&subject, &envelope)
        .await
        .map_err(DomainError::from)
}

/// Wraps a [`RawEventBus`] so it can be used as a domain [`EventPublisher`].
pub struct RawEventBusPublisher<'a>(&'a dyn RawEventBus);

impl<'a> std::fmt::Debug for RawEventBusPublisher<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawEventBusPublisher")
            .finish_non_exhaustive()
    }
}

impl<'a> RawEventBusPublisher<'a> {
    /// Creates a new publisher wrapping the given raw event bus.
    pub fn new(bus: &'a dyn RawEventBus) -> Self {
        Self(bus)
    }
}

#[async_trait::async_trait]
impl EventPublisher for RawEventBusPublisher<'_> {
    async fn publish(&self, event: &Event<DomainEvent>) -> Result<(), DomainError> {
        publish_domain_event(self.0, event).await
    }
}

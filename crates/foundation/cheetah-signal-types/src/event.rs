//! Generic event envelope used by domain and outbox layers.

use crate::{
    CorrelationId, EventId, IdGenerator, MessageId, NodeId, RequestContext, ResourceRef, TenantId,
    UtcTimestamp,
};

/// A generic domain event envelope.
///
/// The payload is typed by the caller so the same envelope can be reused for
/// device, operation, media and messaging events without the foundation crate
/// depending on domain-specific enums.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound = "P: Clone + serde::Serialize + serde::de::DeserializeOwned")]
pub struct Event<P> {
    /// Unique event identifier.
    pub event_id: EventId,
    /// Tenant that owns the aggregate the event belongs to.
    pub tenant_id: TenantId,
    /// Reference to the aggregate that produced the event.
    pub aggregate_ref: ResourceRef,
    /// Monotonic sequence number of the event within the aggregate.
    pub aggregate_sequence: u64,
    /// Wall-clock time when the event occurred.
    pub occurred_at: UtcTimestamp,
    /// Correlation identifier that travels with the request.
    pub correlation_id: CorrelationId,
    /// Causation identifier, usually the request message id.
    pub causation_id: MessageId,
    /// Node that produced the event.
    pub source: NodeId,
    /// Domain-specific payload.
    pub payload: P,
}

impl<P> Event<P> {
    /// Creates a new event from the given context, aggregate reference and payload.
    pub fn new(
        id_generator: &dyn IdGenerator,
        clock: &dyn crate::Clock,
        context: &RequestContext,
        tenant_id: TenantId,
        aggregate_ref: ResourceRef,
        aggregate_sequence: u64,
        payload: P,
    ) -> Self {
        Self {
            event_id: id_generator.generate_event_id(),
            tenant_id,
            aggregate_ref,
            aggregate_sequence,
            occurred_at: clock.now_wall(),
            correlation_id: context.correlation_id,
            causation_id: context.message_id,
            source: context.node_id.unwrap_or_default(),
            payload,
        }
    }

    /// Returns the event payload by reference.
    pub fn payload(&self) -> &P {
        &self.payload
    }

    /// Returns the event payload by mutable reference.
    pub fn payload_mut(&mut self) -> &mut P {
        &mut self.payload
    }

    /// Consumes the event and returns the payload.
    pub fn into_payload(self) -> P {
        self.payload
    }
}

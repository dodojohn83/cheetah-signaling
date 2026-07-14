//! Low-level messaging bus traits.

use cheetah_signal_contracts::cheetah::common::v1::{CommandEnvelope, EventEnvelope};
use prost::Message;

/// Errors that can occur when sending or receiving messages.
#[derive(Debug, thiserror::Error)]
pub enum BusError {
    /// Failed to serialize a domain value to JSON.
    #[error("serialize failed: {0}")]
    Serialize(#[from] serde_json::Error),

    /// Failed to encode a proto envelope.
    #[error("proto encode failed: {0}")]
    Encode(#[from] prost::EncodeError),

    /// Failed to decode a proto envelope.
    #[error("proto decode failed: {0}")]
    Decode(#[from] prost::DecodeError),

    /// The envelope payload is malformed or not the expected kind.
    #[error("invalid payload: {0}")]
    InvalidPayload(String),

    /// The envelope uses a command or event variant that is not supported.
    #[error("unsupported envelope: {0}")]
    UnsupportedEnvelope(String),

    /// The bus is currently overloaded; the caller should back off.
    #[error("bus busy")]
    Busy,

    /// The bus operation timed out.
    #[error("bus timeout")]
    Timeout,

    /// The bus is unavailable.
    #[error("bus unavailable: {0}")]
    Unavailable(String),
}

/// Handle used to acknowledge or negatively acknowledge a delivered message.
#[async_trait::async_trait]
pub trait AckHandle: Send + Sync {
    /// Positively acknowledge the message.
    async fn ack(&self) -> Result<(), BusError>;

    /// Negatively acknowledge the message so it will be redelivered.
    async fn nak(&self, reason: Option<&str>) -> Result<(), BusError>;

    /// Terminate the message (send it to the dead-letter subject).
    async fn term(&self, reason: Option<&str>) -> Result<(), BusError>;
}

/// A single message delivery from a [`Subscription`].
pub struct Delivery<E> {
    /// Decoded envelope.
    pub envelope: E,
    /// Acknowledgement handle for the transport.
    pub ack: Box<dyn AckHandle>,
    /// Unique message id used by the transport for deduplication.
    pub message_id: String,
    /// Subject the message was published to.
    pub subject: String,
}

impl<E> std::fmt::Debug for Delivery<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delivery")
            .field("message_id", &self.message_id)
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

/// A stream of [`Delivery`] items from a message bus.
#[async_trait::async_trait]
pub trait Subscription<E>: Send
where
    E: Message + Send + Default + 'static,
{
    /// Returns the next delivery, or `None` when the subscription is closed.
    async fn next(&mut self) -> Result<Option<Delivery<E>>, BusError>;
}

/// Low-level command bus that exchanges proto [`CommandEnvelope`]s.
#[async_trait::async_trait]
pub trait RawCommandBus: Send + Sync {
    /// Send a command envelope to the given subject.
    async fn send(&self, subject: &str, envelope: &CommandEnvelope) -> Result<(), BusError>;

    /// Subscribe to a subject pattern and receive command envelopes.
    async fn subscribe(
        &self,
        subject: &str,
        consumer_group: &str,
    ) -> Result<Box<dyn Subscription<CommandEnvelope>>, BusError>;
}

/// Low-level event bus that exchanges proto [`EventEnvelope`]s.
#[async_trait::async_trait]
pub trait RawEventBus: Send + Sync {
    /// Publish an event envelope to the given subject.
    async fn publish(&self, subject: &str, envelope: &EventEnvelope) -> Result<(), BusError>;

    /// Subscribe to a subject pattern and receive event envelopes.
    async fn subscribe(
        &self,
        subject: &str,
        consumer_group: &str,
    ) -> Result<Box<dyn Subscription<EventEnvelope>>, BusError>;
}

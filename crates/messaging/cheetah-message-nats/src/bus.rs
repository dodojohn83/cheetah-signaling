//! NATS JetStream command and event bus implementation.

use cheetah_domain::{
    Command, CommandBus, DeviceOwnerResolver, DomainError, DomainEvent, EventPublisher,
};
use cheetah_message_api::{
    AckHandle, BusError, CommandEnvelope, Delivery, EventEnvelope, RawCommandBus, RawEventBus,
    Subscription, command_subject, encode_command,
};
use cheetah_signal_types::{Event, NodeId};
use futures::StreamExt;
use prost::Message;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

const COMMANDS_STREAM: &str = "CHEETAH_COMMANDS";
const EVENTS_STREAM: &str = "CHEETAH_EVENTS";
const COMMAND_SUBJECT_PATTERN: &str = "sig.v1.command.*.*";
const EVENT_SUBJECT_PATTERN: &str = "sig.v1.event.*.*";
/// Maximum timeout passed to `tokio::time::timeout` to avoid `Instant` overflow.
const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

fn clamp_timeout(timeout: Duration) -> Duration {
    timeout.min(MAX_TIMEOUT).max(Duration::from_millis(1))
}

fn nats_error_to_bus(err: impl std::fmt::Display) -> BusError {
    BusError::Unavailable(err.to_string())
}

async fn with_timeout<T, E>(
    timeout: Duration,
    description: &str,
    fut: impl std::future::Future<Output = std::result::Result<T, E>>,
) -> Result<T, BusError>
where
    E: std::fmt::Display,
{
    tokio::time::timeout(clamp_timeout(timeout), fut)
        .await
        .map_err(|_| nats_error_to_bus(format!("{description} timed out")))?
        .map_err(nats_error_to_bus)
}

/// NATS JetStream message bus.
pub struct NatsBus {
    client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
    owner_resolver: Arc<dyn DeviceOwnerResolver>,
    this_node: NodeId,
    operation_timeout: Duration,
}

impl fmt::Debug for NatsBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NatsBus")
            .field("this_node", &self.this_node)
            .finish_non_exhaustive()
    }
}

impl NatsBus {
    /// Connect to a NATS server and ensure the required JetStream streams exist.
    ///
    /// `connect_timeout` bounds the initial TCP/TLS handshake and stream
    /// creation. `operation_timeout` bounds every subsequent publish, subscribe
    /// and ack operation. TLS is required for all cluster communication.
    pub async fn connect(
        url: impl Into<String>,
        this_node: NodeId,
        owner_resolver: Arc<dyn DeviceOwnerResolver>,
        connect_timeout: Duration,
        operation_timeout: Duration,
    ) -> Result<Self, BusError> {
        let url = url.into();
        let scheme = url.split("://").next().unwrap_or(&url);
        if !scheme.eq_ignore_ascii_case("tls") && !scheme.eq_ignore_ascii_case("wss") {
            return Err(nats_error_to_bus(format!(
                "NATS URL must use tls:// or wss:// scheme, got: {url}"
            )));
        }

        let options = async_nats::ConnectOptions::new().require_tls(true);
        let client = with_timeout(
            connect_timeout,
            "NATS connect",
            options.connect(url.as_str()),
        )
        .await?;
        let jetstream = async_nats::jetstream::new(client.clone());

        let commands_config = async_nats::jetstream::stream::Config {
            name: COMMANDS_STREAM.to_string(),
            subjects: vec![COMMAND_SUBJECT_PATTERN.to_string()],
            ..Default::default()
        };
        with_timeout(
            connect_timeout,
            "create NATS commands stream",
            jetstream.get_or_create_stream(commands_config),
        )
        .await?;

        let events_config = async_nats::jetstream::stream::Config {
            name: EVENTS_STREAM.to_string(),
            subjects: vec![EVENT_SUBJECT_PATTERN.to_string()],
            ..Default::default()
        };
        with_timeout(
            connect_timeout,
            "create NATS events stream",
            jetstream.get_or_create_stream(events_config),
        )
        .await?;

        info!("NATS streams created: {COMMANDS_STREAM}, {EVENTS_STREAM}");

        Ok(Self {
            client,
            jetstream,
            owner_resolver,
            this_node,
            operation_timeout,
        })
    }

    /// Returns the NATS client.
    pub fn client(&self) -> &async_nats::Client {
        &self.client
    }

    /// Connect to a NATS server without requiring TLS.
    ///
    /// This is only available under test configuration because production
    /// cluster traffic must be TLS protected.
    #[cfg(any(test, feature = "test-helpers"))]
    pub async fn connect_insecure(
        url: impl Into<String>,
        this_node: NodeId,
        owner_resolver: Arc<dyn DeviceOwnerResolver>,
        connect_timeout: Duration,
        operation_timeout: Duration,
    ) -> Result<Self, BusError> {
        let url = url.into();
        let options = async_nats::ConnectOptions::new();
        let client = with_timeout(
            connect_timeout,
            "NATS connect",
            options.connect(url.as_str()),
        )
        .await?;
        let jetstream = async_nats::jetstream::new(client.clone());

        let commands_config = async_nats::jetstream::stream::Config {
            name: COMMANDS_STREAM.to_string(),
            subjects: vec![COMMAND_SUBJECT_PATTERN.to_string()],
            ..Default::default()
        };
        with_timeout(
            connect_timeout,
            "create NATS commands stream",
            jetstream.get_or_create_stream(commands_config),
        )
        .await?;

        let events_config = async_nats::jetstream::stream::Config {
            name: EVENTS_STREAM.to_string(),
            subjects: vec![EVENT_SUBJECT_PATTERN.to_string()],
            ..Default::default()
        };
        with_timeout(
            connect_timeout,
            "create NATS events stream",
            jetstream.get_or_create_stream(events_config),
        )
        .await?;

        info!("NATS streams created (insecure): {COMMANDS_STREAM}, {EVENTS_STREAM}");

        Ok(Self {
            client,
            jetstream,
            owner_resolver,
            this_node,
            operation_timeout,
        })
    }

    async fn publish_envelope(
        &self,
        subject: &str,
        payload: Vec<u8>,
        message_id: &str,
    ) -> Result<(), BusError> {
        let mut headers = async_nats::HeaderMap::new();
        if !message_id.is_empty() {
            headers.insert("NATS-Msg-Id", message_id);
        }

        let subject = subject.to_string();
        let ack_future = with_timeout(
            self.operation_timeout,
            "NATS publish",
            self.jetstream
                .publish_with_headers(subject, headers, payload.into()),
        )
        .await?;

        let _ = with_timeout(
            self.operation_timeout,
            "NATS publish ack",
            std::future::IntoFuture::into_future(ack_future),
        )
        .await?;
        Ok(())
    }

    fn stream_subjects(stream_name: &str) -> Vec<String> {
        if stream_name == COMMANDS_STREAM {
            vec![COMMAND_SUBJECT_PATTERN.to_string()]
        } else {
            vec![EVENT_SUBJECT_PATTERN.to_string()]
        }
    }

    async fn subscribe_envelope<E>(
        &self,
        stream_name: &str,
        subject: &str,
        consumer_group: &str,
    ) -> Result<Box<dyn Subscription<E>>, BusError>
    where
        E: Message + Send + Default + 'static,
    {
        let stream_config = async_nats::jetstream::stream::Config {
            name: stream_name.to_string(),
            subjects: Self::stream_subjects(stream_name),
            ..Default::default()
        };
        let stream = with_timeout(
            self.operation_timeout,
            "get NATS stream",
            self.jetstream.get_or_create_stream(stream_config),
        )
        .await?;

        let consumer = with_timeout(
            self.operation_timeout,
            "create NATS consumer",
            stream.get_or_create_consumer(
                consumer_group,
                async_nats::jetstream::consumer::pull::Config {
                    durable_name: Some(consumer_group.to_string()),
                    filter_subject: subject.to_string(),
                    ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                    ack_wait: Duration::from_secs(30),
                    max_deliver: 10,
                    deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::All,
                    ..Default::default()
                },
            ),
        )
        .await?;

        let messages = with_timeout(
            self.operation_timeout,
            "open NATS consumer messages",
            consumer.messages(),
        )
        .await?;
        Ok(Box::new(NatsSubscription {
            messages,
            operation_timeout: self.operation_timeout,
            _phantom: std::marker::PhantomData,
        }))
    }
}

#[async_trait::async_trait]
impl RawCommandBus for NatsBus {
    async fn send(&self, subject: &str, envelope: &CommandEnvelope) -> Result<(), BusError> {
        let payload = envelope.encode_to_vec();
        let message_id = envelope
            .meta
            .as_ref()
            .and_then(|m| m.message_id.as_ref())
            .map(|u| u.value.clone())
            .unwrap_or_default();
        self.publish_envelope(subject, payload, &message_id).await
    }

    async fn subscribe(
        &self,
        subject: &str,
        consumer_group: &str,
    ) -> Result<Box<dyn Subscription<CommandEnvelope>>, BusError> {
        self.subscribe_envelope(COMMANDS_STREAM, subject, consumer_group)
            .await
    }
}

#[async_trait::async_trait]
impl RawEventBus for NatsBus {
    async fn publish(&self, subject: &str, envelope: &EventEnvelope) -> Result<(), BusError> {
        let payload = envelope.encode_to_vec();
        let message_id = envelope
            .meta
            .as_ref()
            .and_then(|m| m.message_id.as_ref())
            .map(|u| u.value.clone())
            .unwrap_or_default();
        self.publish_envelope(subject, payload, &message_id).await
    }

    async fn subscribe(
        &self,
        subject: &str,
        consumer_group: &str,
    ) -> Result<Box<dyn Subscription<EventEnvelope>>, BusError> {
        self.subscribe_envelope(EVENTS_STREAM, subject, consumer_group)
            .await
    }
}

#[async_trait::async_trait]
impl CommandBus for NatsBus {
    async fn send(&self, command: &Command) -> cheetah_domain::Result<()> {
        let envelope = encode_command(command).map_err(DomainError::from)?;

        let owner = match self
            .owner_resolver
            .resolve(command.tenant_id(), command.device_id())
            .await
            .map_err(|e| DomainError::unavailable(e.to_string()))?
        {
            Some(owner) => owner,
            None => {
                return Err(DomainError::NoOwner {
                    device_id: command.device_id().to_string(),
                });
            }
        };

        if owner.owner_epoch != command.expected_owner_epoch() {
            return Err(DomainError::StaleOwner {
                expected: command.expected_owner_epoch().0,
                found: owner.owner_epoch.0,
            });
        }

        let subject = command_subject(command.tenant_id(), owner.owner_node_id);
        RawCommandBus::send(self, &subject, &envelope)
            .await
            .map_err(DomainError::from)
    }
}

#[async_trait::async_trait]
impl EventPublisher for NatsBus {
    async fn publish(&self, event: &Event<DomainEvent>) -> cheetah_domain::Result<()> {
        cheetah_message_api::publish_domain_event(self, event).await
    }
}

struct NatsAckHandle {
    acker: async_nats::jetstream::message::Acker,
}

#[async_trait::async_trait]
impl AckHandle for NatsAckHandle {
    async fn ack(&self) -> Result<(), BusError> {
        self.acker.ack().await.map_err(nats_error_to_bus)
    }

    async fn nak(&self, reason: Option<&str>) -> Result<(), BusError> {
        warn!("nacking NATS message: {}", reason.unwrap_or("unknown"));
        self.acker
            .ack_with(async_nats::jetstream::message::AckKind::Nak(None))
            .await
            .map_err(nats_error_to_bus)
    }

    async fn term(&self, reason: Option<&str>) -> Result<(), BusError> {
        warn!("terminating NATS message: {}", reason.unwrap_or("unknown"));
        self.acker
            .ack_with(async_nats::jetstream::message::AckKind::Term)
            .await
            .map_err(nats_error_to_bus)
    }
}

struct NatsSubscription<E> {
    messages: async_nats::jetstream::consumer::pull::Stream,
    operation_timeout: Duration,
    _phantom: std::marker::PhantomData<E>,
}

fn header_message_id(headers: &Option<async_nats::HeaderMap>) -> String {
    headers
        .as_ref()
        .and_then(|h| h.get("NATS-Msg-Id"))
        .map(|v| v.to_string())
        .unwrap_or_default()
}

#[async_trait::async_trait]
impl<E> Subscription<E> for NatsSubscription<E>
where
    E: Message + Send + Default + 'static,
{
    async fn next(&mut self) -> Result<Option<Delivery<E>>, BusError> {
        // Wait for the next message with an operation deadline. A quiet period
        // should not terminate the consumer, so timeouts are retried.
        let message = loop {
            match tokio::time::timeout(clamp_timeout(self.operation_timeout), self.messages.next())
                .await
            {
                Ok(Some(result)) => break result,
                Ok(None) => return Ok(None),
                Err(_) => {
                    tracing::debug!("NATS subscription idle timeout, waiting for next batch");
                }
            }
        };

        match message {
            Ok(message) => {
                let subject = message.subject.to_string();
                let message_id = header_message_id(&message.headers);
                let payload = message.payload.clone();
                let (_, acker) = message.split();

                let envelope = E::decode(&payload[..]).map_err(BusError::Decode)?;
                Ok(Some(Delivery {
                    envelope,
                    ack: Box::new(NatsAckHandle { acker }),
                    message_id,
                    subject,
                }))
            }
            Err(e) => Err(nats_error_to_bus(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_timeout_saturates_at_max_and_min() {
        assert_eq!(clamp_timeout(Duration::ZERO), Duration::from_millis(1));
        assert_eq!(
            clamp_timeout(Duration::from_millis(5_000)),
            Duration::from_millis(5_000)
        );
        assert_eq!(clamp_timeout(Duration::MAX), MAX_TIMEOUT);
    }
}

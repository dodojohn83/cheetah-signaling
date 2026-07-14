//! NATS JetStream command and event bus implementation.

use cheetah_domain::{Command, CommandBus, DeviceOwnerResolver, DomainError, DomainEvent, EventPublisher};
use cheetah_message_api::{
    AckHandle, BusError, CommandEnvelope, Delivery, EventEnvelope, RawCommandBus, RawEventBus,
    Subscription, command_subject, encode_command, encode_event, event_subject,
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

fn nats_error_to_bus(err: impl std::fmt::Display) -> BusError {
    BusError::Unavailable(err.to_string())
}

fn bus_error_to_domain(err: BusError) -> DomainError {
    match err {
        BusError::Busy => DomainError::unavailable("message bus busy"),
        BusError::Unavailable(msg) => DomainError::unavailable(msg),
        BusError::InvalidPayload(msg) | BusError::UnsupportedEnvelope(msg) => {
            DomainError::invalid_argument(msg)
        }
        _ => DomainError::internal(err.to_string()),
    }
}

/// NATS JetStream message bus.
pub struct NatsBus {
    client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
    owner_resolver: Arc<dyn DeviceOwnerResolver>,
    this_node: NodeId,
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
    pub async fn connect(
        url: impl Into<String>,
        this_node: NodeId,
        owner_resolver: Arc<dyn DeviceOwnerResolver>,
    ) -> Result<Self, BusError> {
        let client = async_nats::connect(url.into()).await.map_err(nats_error_to_bus)?;
        let jetstream = async_nats::jetstream::new(client.clone());

        jetstream
            .get_or_create_stream(async_nats::jetstream::stream::Config {
                name: COMMANDS_STREAM.to_string(),
                subjects: vec![COMMAND_SUBJECT_PATTERN.to_string()],
                ..Default::default()
            })
            .await
            .map_err(nats_error_to_bus)?;

        jetstream
            .get_or_create_stream(async_nats::jetstream::stream::Config {
                name: EVENTS_STREAM.to_string(),
                subjects: vec![EVENT_SUBJECT_PATTERN.to_string()],
                ..Default::default()
            })
            .await
            .map_err(nats_error_to_bus)?;

        info!("NATS streams created: {COMMANDS_STREAM}, {EVENTS_STREAM}");

        Ok(Self {
            client,
            jetstream,
            owner_resolver,
            this_node,
        })
    }

    /// Returns the NATS client.
    pub fn client(&self) -> &async_nats::Client {
        &self.client
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
        let ack_future = self
            .jetstream
            .publish_with_headers(subject, headers, payload.into())
            .await
            .map_err(|e| BusError::Unavailable(e.to_string()))?;

        let _ = ack_future.await.map_err(nats_error_to_bus)?;
        Ok(())
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
        let stream = self
            .jetstream
            .get_or_create_stream(async_nats::jetstream::stream::Config {
                name: stream_name.to_string(),
                ..Default::default()
            })
            .await
            .map_err(nats_error_to_bus)?;

        let consumer = stream
            .get_or_create_consumer(consumer_group, async_nats::jetstream::consumer::pull::Config {
                durable_name: Some(consumer_group.to_string()),
                filter_subject: subject.to_string(),
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                max_deliver: 10,
                deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::All,
                ..Default::default()
            })
            .await
            .map_err(nats_error_to_bus)?;

        let messages = consumer.messages().await.map_err(nats_error_to_bus)?;
        Ok(Box::new(NatsSubscription {
            messages,
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
        self.subscribe_envelope(COMMANDS_STREAM, subject, consumer_group).await
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
        self.subscribe_envelope(EVENTS_STREAM, subject, consumer_group).await
    }
}

#[async_trait::async_trait]
impl CommandBus for NatsBus {
    async fn send(&self, command: &Command) -> cheetah_domain::Result<()> {
        let envelope = encode_command(command).map_err(bus_error_to_domain)?;

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
            .map_err(bus_error_to_domain)
    }
}

#[async_trait::async_trait]
impl EventPublisher for NatsBus {
    async fn publish(&self, event: &Event<DomainEvent>) -> cheetah_domain::Result<()> {
        let envelope = encode_event(event).map_err(bus_error_to_domain)?;
        let subject = event_subject(event.tenant_id, "domain_event");
        RawEventBus::publish(self, &subject, &envelope)
            .await
            .map_err(bus_error_to_domain)
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
        match self.messages.next().await {
            Some(Ok(message)) => {
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
            Some(Err(e)) => Err(nats_error_to_bus(e)),
            None => Ok(None),
        }
    }
}

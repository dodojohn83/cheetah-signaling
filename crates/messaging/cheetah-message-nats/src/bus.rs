//! NATS JetStream command and event bus implementation.

use cheetah_domain::{
    Command, CommandBus, DeviceOwnerResolver, DomainError, DomainEvent, EventPublisher,
};
use cheetah_message_api::{
    AckHandle, BusError, CommandEnvelope, Delivery, EventEnvelope, RawCommandBus, RawEventBus,
    Subscription, command_subject, encode_command, encode_event, event_subject,
};
use cheetah_signal_types::config::{MessagingConfig, NatsAuth};
use cheetah_signal_types::{Event, NodeId, SecretStore};
use futures::StreamExt;
use prost::Message;
use secrecy::ExposeSecret;
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

async fn with_timeout<T, E>(
    timeout: Duration,
    description: &str,
    fut: impl std::future::Future<Output = std::result::Result<T, E>>,
) -> Result<T, BusError>
where
    E: std::fmt::Display,
{
    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| nats_error_to_bus(format!("{description} timed out")))?
        .map_err(nats_error_to_bus)
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
    operation_timeout: Duration,
    publish_allow: Vec<String>,
    subscribe_allow: Vec<String>,
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
    ///
    /// When `config.nats_auth` is set, the referenced secrets are resolved from
    /// `secret_store` and used to authenticate the client. Subject permissions
    /// are enforced in-process before any publish or subscribe, defaulting to the
    /// command/event subjects required by this node.
    pub async fn connect(
        config: &MessagingConfig,
        this_node: NodeId,
        owner_resolver: Arc<dyn DeviceOwnerResolver>,
        secret_store: Option<Arc<dyn SecretStore>>,
        connect_timeout: Duration,
        operation_timeout: Duration,
    ) -> Result<Self, BusError> {
        let url = &config.nats_url;
        let scheme = url.split("://").next().unwrap_or(url).to_lowercase();
        if !matches!(scheme.as_str(), "tls" | "wss") {
            return Err(nats_error_to_bus(format!(
                "NATS URL must use tls:// or wss:// scheme, got: {url}"
            )));
        }

        let mut options = async_nats::ConnectOptions::new().require_tls(true);

        if let Some(auth) = config.nats_auth.as_ref() {
            let secret_store = secret_store.ok_or_else(|| {
                BusError::InvalidPayload(
                    "NATS authentication configured but no secret store provided".to_string(),
                )
            })?;
            options = match auth {
                NatsAuth::Token { token_ref } => {
                    let token = secret_store.get(token_ref).map_err(|e| {
                        BusError::InvalidPayload(format!("failed to load NATS token: {e}"))
                    })?;
                    options.token(token.expose_secret().to_string())
                }
                NatsAuth::UserAndPassword {
                    username,
                    password_ref,
                } => {
                    let password = secret_store.get(password_ref).map_err(|e| {
                        BusError::InvalidPayload(format!("failed to load NATS password: {e}"))
                    })?;
                    options
                        .user_and_password(username.clone(), password.expose_secret().to_string())
                }
            };
        }

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

        let (publish_allow, subscribe_allow) =
            Self::resolve_permissions(config.nats_permissions.as_ref(), &this_node);

        Ok(Self {
            client,
            jetstream,
            owner_resolver,
            this_node,
            operation_timeout,
            publish_allow,
            subscribe_allow,
        })
    }

    /// Resolve effective publish/subscribe allow-lists.
    ///
    /// If the configuration does not specify permissions, a least-privilege
    /// default is derived from this node's identity.
    fn resolve_permissions(
        configured: Option<&cheetah_signal_types::config::NatsPermissions>,
        this_node: &NodeId,
    ) -> (Vec<String>, Vec<String>) {
        if let Some(p) = configured {
            let publish = if p.publish_allow.is_empty() {
                Self::default_publish_patterns()
            } else {
                p.publish_allow.clone()
            };
            let subscribe = if p.subscribe_allow.is_empty() {
                Self::default_subscribe_patterns(this_node)
            } else {
                p.subscribe_allow.clone()
            };
            (publish, subscribe)
        } else {
            (
                Self::default_publish_patterns(),
                Self::default_subscribe_patterns(this_node),
            )
        }
    }

    fn default_publish_patterns() -> Vec<String> {
        vec!["sig.v1.command.>".to_string(), "sig.v1.event.>".to_string()]
    }

    fn default_subscribe_patterns(this_node: &NodeId) -> Vec<String> {
        vec![
            format!("sig.v1.command.*.{this_node}"),
            "sig.v1.event.>".to_string(),
        ]
    }

    /// Checks whether `subject` is allowed by any of `patterns`.
    ///
    /// Supports NATS subject wildcards: `*` matches a single token and `>`
    /// matches zero or more remaining tokens at the end of a pattern.
    fn subject_matches(subject: &str, pattern: &str) -> bool {
        let subject_tokens: Vec<&str> = subject.split('.').collect();
        let pattern_tokens: Vec<&str> = pattern.split('.').collect();

        for (idx, pat) in pattern_tokens.iter().enumerate() {
            if *pat == ">" {
                return idx == pattern_tokens.len() - 1;
            }
            if let Some(sub) = subject_tokens.get(idx) {
                if *pat != "*" && *pat != *sub {
                    return false;
                }
            } else {
                return false;
            }
        }

        subject_tokens.len() == pattern_tokens.len()
    }

    fn is_subject_allowed(subject: &str, patterns: &[String]) -> bool {
        patterns.iter().any(|p| Self::subject_matches(subject, p))
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
        if !Self::is_subject_allowed(subject, &self.publish_allow) {
            return Err(BusError::InvalidPayload(format!(
                "publish subject {subject} is not in the allowed list"
            )));
        }

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
        if !Self::is_subject_allowed(subject, &self.subscribe_allow) {
            return Err(BusError::InvalidPayload(format!(
                "subscribe subject {subject} is not in the allowed list"
            )));
        }

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
            match tokio::time::timeout(self.operation_timeout, self.messages.next()).await {
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
    fn subject_match_exact() {
        assert!(NatsBus::subject_matches(
            "sig.v1.command.3f.node-1",
            "sig.v1.command.3f.node-1"
        ));
    }

    #[test]
    fn subject_match_single_wildcard() {
        assert!(NatsBus::subject_matches(
            "sig.v1.command.3f.node-1",
            "sig.v1.command.*.node-1"
        ));
        assert!(!NatsBus::subject_matches(
            "sig.v1.command.3f.extra.node-1",
            "sig.v1.command.*.node-1"
        ));
    }

    #[test]
    fn subject_match_multi_wildcard() {
        assert!(NatsBus::subject_matches(
            "sig.v1.command.3f.node-1",
            "sig.v1.command.>"
        ));
        assert!(NatsBus::subject_matches(
            "sig.v1.command.3f.node-1.extra",
            "sig.v1.command.>"
        ));
        assert!(NatsBus::subject_matches(
            "sig.v1.event.3f.domain_event",
            "sig.v1.event.>"
        ));
    }

    #[test]
    fn subject_match_rejects_mismatch() {
        assert!(!NatsBus::subject_matches(
            "sig.v1.event.3f.domain_event",
            "sig.v1.command.>"
        ));
        assert!(!NatsBus::subject_matches(
            "sig.v1.command.3f.node-1",
            "sig.v1.command.*"
        ));
    }

    #[test]
    fn is_subject_allowed_any_pattern() {
        let patterns = vec![
            "sig.v1.command.>".to_string(),
            "sig.v1.event.*.*".to_string(),
        ];
        assert!(NatsBus::is_subject_allowed(
            "sig.v1.command.3f.node-1",
            &patterns
        ));
        assert!(NatsBus::is_subject_allowed(
            "sig.v1.event.3f.domain_event",
            &patterns
        ));
        assert!(!NatsBus::is_subject_allowed(
            "unauthorized.subject",
            &patterns
        ));
    }
}

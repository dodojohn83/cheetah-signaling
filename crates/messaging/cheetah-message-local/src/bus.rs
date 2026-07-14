//! In-process message bus backed by bounded `tokio` channels.

use cheetah_domain::{Command, CommandBus, DomainError, DomainEvent, EventPublisher};
use cheetah_message_api::{
    AckHandle, BusError, CommandEnvelope, Delivery, EventEnvelope, RawCommandBus, RawEventBus,
    Subscription, encode_command, encode_event,
};
use cheetah_signal_types::Event;
use std::sync::{Mutex, MutexGuard};
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

fn lock_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
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

/// In-process command/event bus that serializes through the same proto
/// envelopes as the NATS implementation.
#[derive(Debug)]
pub struct InProcessMessageBus {
    command_tx: mpsc::Sender<CommandEnvelope>,
    command_rx: Mutex<Option<mpsc::Receiver<CommandEnvelope>>>,
    event_tx: broadcast::Sender<EventEnvelope>,
    command_capacity: usize,
    event_capacity: usize,
}

impl InProcessMessageBus {
    /// Creates a new bus with the given per-channel capacity limits.
    pub fn new(command_capacity: usize, event_capacity: usize) -> Self {
        let (command_tx, command_rx) = mpsc::channel(command_capacity);
        let (event_tx, _) = broadcast::channel(event_capacity);
        Self {
            command_tx,
            command_rx: Mutex::new(Some(command_rx)),
            event_tx,
            command_capacity,
            event_capacity,
        }
    }

    /// Returns the configured command channel capacity.
    pub fn command_capacity(&self) -> usize {
        self.command_capacity
    }

    /// Returns the configured event channel capacity.
    pub fn event_capacity(&self) -> usize {
        self.event_capacity
    }

    async fn do_send_command(&self, _subject: &str, envelope: &CommandEnvelope) -> Result<(), BusError> {
        self.command_tx
            .try_send(envelope.clone())
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => BusError::Busy,
                mpsc::error::TrySendError::Closed(_) => {
                    BusError::Unavailable("command channel closed".to_string())
                }
            })
    }

    async fn do_publish_event(&self, _subject: &str, envelope: &EventEnvelope) -> Result<(), BusError> {
        if self.event_tx.receiver_count() == 0 {
            return Ok(());
        }
        match self.event_tx.send(envelope.clone()) {
            Ok(_) => Ok(()),
            Err(broadcast::error::SendError { .. }) => {
                // All receivers dropped between the check and send.
                Ok(())
            }
        }
    }
}

#[async_trait::async_trait]
impl RawCommandBus for InProcessMessageBus {
    async fn send(&self, subject: &str, envelope: &CommandEnvelope) -> Result<(), BusError> {
        self.do_send_command(subject, envelope).await
    }

    async fn subscribe(
        &self,
        _subject: &str,
        _consumer_group: &str,
    ) -> Result<Box<dyn Subscription<CommandEnvelope>>, BusError> {
        let receiver = lock_mutex(&self.command_rx)
            .take()
            .ok_or_else(|| BusError::Unavailable("command subscription already taken".to_string()))?;
        Ok(Box::new(CommandSubscription { receiver }))
    }
}

#[async_trait::async_trait]
impl RawEventBus for InProcessMessageBus {
    async fn publish(&self, subject: &str, envelope: &EventEnvelope) -> Result<(), BusError> {
        self.do_publish_event(subject, envelope).await
    }

    async fn subscribe(
        &self,
        _subject: &str,
        _consumer_group: &str,
    ) -> Result<Box<dyn Subscription<EventEnvelope>>, BusError> {
        let receiver = self.event_tx.subscribe();
        Ok(Box::new(EventSubscription { receiver }))
    }
}

#[async_trait::async_trait]
impl CommandBus for InProcessMessageBus {
    async fn send(&self, command: &Command) -> cheetah_domain::Result<()> {
        let envelope = encode_command(command).map_err(bus_error_to_domain)?;
        RawCommandBus::send(self, "", &envelope)
            .await
            .map_err(bus_error_to_domain)
    }
}

#[async_trait::async_trait]
impl EventPublisher for InProcessMessageBus {
    async fn publish(&self, event: &Event<DomainEvent>) -> cheetah_domain::Result<()> {
        let envelope = encode_event(event).map_err(bus_error_to_domain)?;
        RawEventBus::publish(self, "", &envelope)
            .await
            .map_err(bus_error_to_domain)
    }
}

/// No-op acknowledgement handle for the in-process bus.
#[derive(Debug, Clone, Copy)]
struct NoopAck;

#[async_trait::async_trait]
impl AckHandle for NoopAck {
    async fn ack(&self) -> Result<(), BusError> {
        Ok(())
    }

    async fn nak(&self, _reason: Option<&str>) -> Result<(), BusError> {
        Ok(())
    }

    async fn term(&self, _reason: Option<&str>) -> Result<(), BusError> {
        Ok(())
    }
}

fn message_id(envelope: &CommandEnvelope) -> String {
    envelope
        .meta
        .as_ref()
        .and_then(|m| m.message_id.as_ref())
        .map(|u| u.value.clone())
        .unwrap_or_default()
}

fn event_message_id(envelope: &EventEnvelope) -> String {
    envelope
        .meta
        .as_ref()
        .and_then(|m| m.message_id.as_ref())
        .map(|u| u.value.clone())
        .unwrap_or_default()
}

struct CommandSubscription {
    receiver: mpsc::Receiver<CommandEnvelope>,
}

#[async_trait::async_trait]
impl Subscription<CommandEnvelope> for CommandSubscription {
    async fn next(&mut self) -> Result<Option<Delivery<CommandEnvelope>>, BusError> {
        match self.receiver.recv().await {
            Some(envelope) => {
                let id = message_id(&envelope);
                Ok(Some(Delivery {
                    envelope,
                    ack: Box::new(NoopAck),
                    message_id: id,
                    subject: String::new(),
                }))
            }
            None => Ok(None),
        }
    }
}

struct EventSubscription {
    receiver: broadcast::Receiver<EventEnvelope>,
}

#[async_trait::async_trait]
impl Subscription<EventEnvelope> for EventSubscription {
    async fn next(&mut self) -> Result<Option<Delivery<EventEnvelope>>, BusError> {
        loop {
            match self.receiver.recv().await {
                Ok(envelope) => {
                    let id = event_message_id(&envelope);
                    return Ok(Some(Delivery {
                        envelope,
                        ack: Box::new(NoopAck),
                        message_id: id,
                        subject: String::new(),
                    }));
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    warn!("event subscription lagged by {count} messages");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return Ok(None),
            }
        }
    }
}

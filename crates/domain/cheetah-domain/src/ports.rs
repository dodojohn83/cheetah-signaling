//! Domain and application ports.

use crate::{
    Channel, Command, Device, DomainError, DomainEvent, MediaBinding, MediaSession, Operation,
};
use cheetah_signal_types::{
    ChannelId, DeviceId, Event, EventId, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId,
    MessageId, NodeId, OperationId, OwnerEpoch, ProtocolIdentity, TenantId, UtcTimestamp,
};

pub use cheetah_signal_types::{Clock, IdGenerator};

/// Result alias for port operations.
pub type Result<T> = std::result::Result<T, DomainError>;

/// Information about the current owner of a device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerInfo {
    /// Node that owns the device.
    pub owner_node_id: NodeId,
    /// Owner epoch.
    pub owner_epoch: OwnerEpoch,
    /// Lease expiration time. `None` means the lease never expires.
    pub lease_until: Option<cheetah_signal_types::UtcTimestamp>,
}

/// Reservation of a media node resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaReservation {
    /// Media node that will host the binding.
    pub media_node_id: NodeId,
    /// Instance epoch of the media node.
    pub media_node_instance_epoch: MediaNodeInstanceEpoch,
}

/// A single outbox entry.
#[derive(Clone, Debug, PartialEq)]
pub struct OutboxEntry {
    /// Event envelope.
    pub event: Event<DomainEvent>,
    /// Whether the event has been published.
    pub published: bool,
    /// Number of publish attempts made so far.
    pub attempts: u32,
    /// Whether the event has entered the permanent failure state.
    pub failed: bool,
    /// Optional human-readable failure reason.
    pub error: Option<String>,
    /// Earliest time at which the event may be retried.
    pub next_attempt_at: Option<UtcTimestamp>,
}

/// Repository for device aggregates.
#[async_trait::async_trait]
pub trait DeviceRepository: Send {
    /// Gets a device by id.
    async fn get(&mut self, tenant_id: TenantId, device_id: DeviceId) -> Result<Option<Device>>;
    /// Gets a device by protocol external identity.
    async fn get_by_external_id(
        &mut self,
        tenant_id: TenantId,
        protocol: crate::Protocol,
        external_id: ProtocolIdentity,
    ) -> Result<Option<Device>>;
    /// Saves a device.
    async fn save(&mut self, device: &Device) -> Result<()>;
}

/// Repository for channel aggregates.
#[async_trait::async_trait]
pub trait ChannelRepository: Send {
    /// Gets a channel by id.
    async fn get(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
    ) -> Result<Option<Channel>>;
    /// Lists all channels for a device.
    async fn list_by_device(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Vec<Channel>>;
    /// Saves a channel.
    async fn save(&mut self, channel: &Channel) -> Result<()>;
    /// Removes a channel.
    async fn remove(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
    ) -> Result<()>;
}

/// Repository for operation aggregates.
#[async_trait::async_trait]
pub trait OperationRepository: Send {
    /// Gets an operation by id.
    async fn get(
        &mut self,
        tenant_id: TenantId,
        operation_id: OperationId,
    ) -> Result<Option<Operation>>;
    /// Gets an operation by idempotency scope.
    async fn get_by_idempotency(
        &mut self,
        scope: &crate::IdempotencyScope,
    ) -> Result<Option<Operation>>;
    /// Saves an operation.
    async fn save(&mut self, operation: &Operation) -> Result<()>;
}

/// Repository for media session aggregates.
#[async_trait::async_trait]
pub trait MediaSessionRepository: Send {
    /// Gets a media session by id.
    async fn get(
        &mut self,
        tenant_id: TenantId,
        media_session_id: MediaSessionId,
    ) -> Result<Option<MediaSession>>;
    /// Gets a media session by idempotency scope.
    async fn get_by_idempotency(
        &mut self,
        scope: &crate::IdempotencyScope,
    ) -> Result<Option<MediaSession>>;
    /// Saves a media session.
    async fn save(&mut self, session: &MediaSession) -> Result<()>;
}

/// Repository for media binding aggregates.
#[async_trait::async_trait]
pub trait MediaBindingRepository: Send {
    /// Gets a media binding by id.
    async fn get(
        &mut self,
        tenant_id: TenantId,
        media_binding_id: MediaBindingId,
    ) -> Result<Option<MediaBinding>>;
    /// Gets a media binding by media session id.
    async fn get_by_media_session(
        &mut self,
        tenant_id: TenantId,
        media_session_id: MediaSessionId,
    ) -> Result<Option<MediaBinding>>;
    /// Saves a media binding.
    async fn save(&mut self, binding: &MediaBinding) -> Result<()>;
}

/// Outbox for domain events.
#[async_trait::async_trait]
pub trait Outbox: Send {
    /// Appends an event to the outbox.
    async fn append(&mut self, event: Event<DomainEvent>) -> Result<()>;
    /// Returns pending events up to `limit`.
    ///
    /// Pending events are those with `published = 0`, `failed = 0`, and a
    /// `next_attempt_at` that is `NULL` or in the past relative to `now`.
    async fn pending(&mut self, now: UtcTimestamp, limit: usize) -> Result<Vec<OutboxEntry>>;
    /// Marks an event as published.
    async fn mark_published(&mut self, event_id: EventId) -> Result<()>;
    /// Marks an event as failed and schedules the next retry attempt.
    ///
    /// When `attempts` reaches the configured maximum the caller should set
    /// `failed` to `true` and provide a permanent failure reason.
    async fn mark_failed(
        &mut self,
        event_id: EventId,
        attempts: u32,
        failed: bool,
        error: Option<String>,
        next_attempt_at: Option<UtcTimestamp>,
    ) -> Result<()>;
}

/// Publishes events to the message bus.
#[async_trait::async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publishes a single event.
    async fn publish(&self, event: &Event<DomainEvent>) -> Result<()>;
}

/// Sends commands to the protocol driver or plugin.
#[async_trait::async_trait]
pub trait CommandBus: Send + Sync {
    /// Sends a command.
    async fn send(&self, command: &Command) -> Result<()>;
}

/// Resolves the current owner of a device.
#[async_trait::async_trait]
pub trait DeviceOwnerResolver: Send + Sync {
    /// Resolves owner information for a device.
    async fn resolve(&self, tenant_id: TenantId, device_id: DeviceId) -> Result<Option<OwnerInfo>>;
}

/// Reserves media node resources for media sessions.
#[async_trait::async_trait]
pub trait MediaPort: Send + Sync {
    /// Reserves a media node for a live session.
    async fn reserve_live(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
        media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        purpose: crate::MediaPurpose,
    ) -> Result<MediaReservation>;

    /// Reserves a media node for a playback session.
    #[allow(clippy::too_many_arguments)]
    async fn reserve_playback(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
        media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        start_time: cheetah_signal_types::UtcTimestamp,
        end_time: cheetah_signal_types::UtcTimestamp,
        scale: f64,
    ) -> Result<MediaReservation>;

    /// Reserves a media node for a talk session.
    async fn reserve_talk(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
        media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
    ) -> Result<MediaReservation>;

    /// Releases a media binding.
    async fn release(&self, tenant_id: TenantId, media_binding_id: MediaBindingId) -> Result<()>;
}

/// Status of an idempotent inbox record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessedMessageStatus {
    /// The message has been accepted but not yet processed.
    Pending,
    /// The message was processed successfully.
    Completed,
    /// Processing the message failed.
    Failed,
    /// The message was a duplicate of an earlier processed message.
    Duplicate,
}

impl std::fmt::Display for ProcessedMessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessedMessageStatus::Pending => write!(f, "pending"),
            ProcessedMessageStatus::Completed => write!(f, "completed"),
            ProcessedMessageStatus::Failed => write!(f, "failed"),
            ProcessedMessageStatus::Duplicate => write!(f, "duplicate"),
        }
    }
}

/// An idempotent inbox record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessedMessageRecord {
    /// Tenant identifier.
    pub tenant_id: TenantId,
    /// Message identifier.
    pub message_id: MessageId,
    /// Optional idempotency key supplied by the sender.
    pub idempotency_key: Option<String>,
    /// Current status.
    pub status: ProcessedMessageStatus,
    /// Optional JSON-encoded result payload.
    pub result_payload: Option<String>,
    /// Time at which the record was created or completed.
    pub processed_at: UtcTimestamp,
    /// Time at which the record may be evicted.
    pub expires_at: Option<UtcTimestamp>,
}

/// Repository for idempotent inbox records.
#[async_trait::async_trait]
pub trait ProcessedMessageRepository: Send {
    /// Finds an existing record.
    async fn find(
        &mut self,
        tenant_id: TenantId,
        message_id: MessageId,
    ) -> Result<Option<ProcessedMessageRecord>>;

    /// Inserts `record` if no record exists for `(tenant_id, message_id)` and
    /// returns `None`; otherwise returns the existing record.
    async fn get_or_insert(
        &mut self,
        record: ProcessedMessageRecord,
    ) -> Result<Option<ProcessedMessageRecord>>;

    /// Marks the record as completed (or failed) with a result payload.
    async fn complete(
        &mut self,
        tenant_id: TenantId,
        message_id: MessageId,
        status: ProcessedMessageStatus,
        result_payload: Option<String>,
        processed_at: UtcTimestamp,
    ) -> Result<()>;
}

/// Unit of work that keeps aggregate and outbox writes in one transaction.
#[async_trait::async_trait]
pub trait UnitOfWork: Send {
    /// Access the device repository.
    fn device_repository(&mut self) -> &mut dyn DeviceRepository;
    /// Access the channel repository.
    fn channel_repository(&mut self) -> &mut dyn ChannelRepository;
    /// Access the operation repository.
    fn operation_repository(&mut self) -> &mut dyn OperationRepository;
    /// Access the media session repository.
    fn media_session_repository(&mut self) -> &mut dyn MediaSessionRepository;
    /// Access the media binding repository.
    fn media_binding_repository(&mut self) -> &mut dyn MediaBindingRepository;
    /// Access the processed message repository.
    fn processed_message_repository(&mut self) -> &mut dyn ProcessedMessageRepository;
    /// Access the outbox.
    fn outbox(&mut self) -> &mut dyn Outbox;
    /// Commit the unit of work.
    async fn commit(&mut self) -> Result<()>;
    /// Rollback the unit of work.
    async fn rollback(&mut self) -> Result<()>;
}

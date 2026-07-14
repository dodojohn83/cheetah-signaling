//! Domain and application ports.

use crate::{
    Channel, Command, Device, DomainError, DomainEvent, MediaBinding, MediaSession, Operation,
};
use cheetah_signal_types::{
    ChannelId, DeviceId, Event, EventId, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId,
    NodeId, OperationId, OwnerEpoch, ProtocolIdentity, TenantId,
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
    async fn pending(&mut self, limit: usize) -> Result<Vec<OutboxEntry>>;
    /// Marks an event as published.
    async fn mark_published(&mut self, event_id: EventId) -> Result<()>;
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
    /// Access the outbox.
    fn outbox(&mut self) -> &mut dyn Outbox;
    /// Commit the unit of work.
    async fn commit(&mut self) -> Result<()>;
    /// Rollback the unit of work.
    async fn rollback(&mut self) -> Result<()>;
}

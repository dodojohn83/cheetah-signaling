//! In-memory test helpers for `cheetah-domain`.
//!
//! These implementations are deterministic and safe to use from synchronous or
//! asynchronous tests. They use `std::sync::Mutex` and never hold a lock across
//! an `.await` point.

use crate::{
    Channel, ChannelRepository, Command, CommandBus, Device, DeviceRepository, DomainError,
    DomainEvent, EventPublisher, MediaBinding, MediaBindingRepository, MediaReservation,
    MediaSession, MediaSessionRepository, Operation, OperationRepository, Outbox, OutboxEntry,
    OwnerInfo, UnitOfWork,
};
use cheetah_signal_types::{
    ChannelId, Clock, DeviceId, DurationMs, Event, IdGenerator, MediaBindingId,
    MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, OperationId, Principal,
    ProtocolIdentity, RequestContext, ResourceId, ResourceKind, ResourceRef, TenantId,
    UtcTimestamp,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use time::{Duration as TimeDuration, OffsetDateTime};

/// Locks a mutex, recovering from poisoning without panicking.
fn lock_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Test clock with explicit wall and monotonic time.
#[derive(Debug)]
pub struct InMemoryClock {
    wall_ms: AtomicU64,
    mono_ms: AtomicU64,
}

impl InMemoryClock {
    /// Creates a new clock starting at the Unix epoch.
    pub fn new() -> Self {
        Self {
            wall_ms: AtomicU64::new(0),
            mono_ms: AtomicU64::new(0),
        }
    }

    /// Advances the clock by the given duration.
    pub fn advance(&self, duration: DurationMs) {
        let ms = duration.as_millis() as u64;
        self.wall_ms.fetch_add(ms, Ordering::SeqCst);
        self.mono_ms.fetch_add(ms, Ordering::SeqCst);
    }

    /// Sets the wall time to the given number of milliseconds from the Unix epoch.
    pub fn set_wall_ms(&self, ms: u64) {
        self.wall_ms.store(ms, Ordering::SeqCst);
    }

    /// Sets the monotonic time to the given number of milliseconds.
    pub fn set_mono_ms(&self, ms: u64) {
        self.mono_ms.store(ms, Ordering::SeqCst);
    }
}

impl Default for InMemoryClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for InMemoryClock {
    fn now_wall(&self) -> UtcTimestamp {
        let ms = self.wall_ms.load(Ordering::SeqCst) as i64;
        UtcTimestamp::from_offset(OffsetDateTime::UNIX_EPOCH + TimeDuration::milliseconds(ms))
    }

    fn now_monotonic(&self) -> DurationMs {
        let ms = self.mono_ms.load(Ordering::SeqCst) as i64;
        DurationMs::from_millis(ms)
    }
}

/// Test id generator that produces deterministic, non-nil UUIDs.
#[derive(Debug)]
pub struct InMemoryIdGenerator {
    counter: Arc<AtomicU64>,
}

impl InMemoryIdGenerator {
    /// Creates a new id generator.
    pub fn new() -> Self {
        Self {
            counter: Arc::new(AtomicU64::new(1)),
        }
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst)
    }

    fn next_uuid(&self) -> uuid::Uuid {
        let n = self.next();
        uuid::Uuid::from_u64_pair(0, n)
    }
}

impl Default for InMemoryIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl IdGenerator for InMemoryIdGenerator {
    fn generate_tenant_id(&self) -> TenantId {
        TenantId::from_uuid(self.next_uuid())
    }

    fn generate_device_id(&self) -> DeviceId {
        DeviceId::from_uuid(self.next_uuid())
    }

    fn generate_endpoint_id(&self) -> cheetah_signal_types::EndpointId {
        cheetah_signal_types::EndpointId::from_uuid(self.next_uuid())
    }

    fn generate_channel_id(&self) -> ChannelId {
        ChannelId::from_uuid(self.next_uuid())
    }

    fn generate_protocol_session_id(&self) -> cheetah_signal_types::ProtocolSessionId {
        cheetah_signal_types::ProtocolSessionId::from_uuid(self.next_uuid())
    }

    fn generate_media_session_id(&self) -> MediaSessionId {
        MediaSessionId::from_uuid(self.next_uuid())
    }

    fn generate_media_binding_id(&self) -> MediaBindingId {
        MediaBindingId::from_uuid(self.next_uuid())
    }

    fn generate_media_node_instance_epoch(&self) -> MediaNodeInstanceEpoch {
        MediaNodeInstanceEpoch(self.next())
    }

    fn generate_operation_id(&self) -> OperationId {
        OperationId::from_uuid(self.next_uuid())
    }

    fn generate_node_id(&self) -> NodeId {
        NodeId::from_uuid(self.next_uuid())
    }

    fn generate_plugin_id(&self) -> cheetah_signal_types::PluginId {
        cheetah_signal_types::PluginId::from_uuid(self.next_uuid())
    }

    fn generate_event_id(&self) -> cheetah_signal_types::EventId {
        cheetah_signal_types::EventId::from_uuid(self.next_uuid())
    }

    fn generate_message_id(&self) -> MessageId {
        MessageId::from_uuid(self.next_uuid())
    }

    fn generate_correlation_id(&self) -> cheetah_signal_types::CorrelationId {
        cheetah_signal_types::CorrelationId::from_uuid(self.next_uuid())
    }
}

/// In-memory stores used by the unit of work and other in-memory adapters.
#[derive(Clone, Debug, Default)]
pub struct InMemoryStores {
    /// Stored devices keyed by `(tenant_id, device_id)`.
    pub devices: BTreeMap<(TenantId, DeviceId), Device>,
    /// Stored channels keyed by `(tenant_id, device_id, channel_id)`.
    pub channels: BTreeMap<(TenantId, DeviceId, ChannelId), Channel>,
    /// Stored operations keyed by `(tenant_id, operation_id)`.
    pub operations: BTreeMap<(TenantId, OperationId), Operation>,
    /// Stored media sessions keyed by `(tenant_id, media_session_id)`.
    pub sessions: BTreeMap<(TenantId, MediaSessionId), MediaSession>,
    /// Stored media bindings keyed by `(tenant_id, media_binding_id)`.
    pub bindings: BTreeMap<(TenantId, MediaBindingId), MediaBinding>,
    /// Outbox entries.
    pub outbox: Vec<OutboxEntry>,
    /// Commands dispatched through the in-memory command bus.
    pub commands: Vec<Command>,
    /// Events published through the in-memory event publisher.
    pub published_events: Vec<Event<DomainEvent>>,
    /// Owner map used by the in-memory owner resolver.
    pub owners: BTreeMap<(TenantId, DeviceId), OwnerInfo>,
    /// Media reservations keyed by `(tenant_id, media_binding_id)`.
    pub media_reservations: BTreeMap<(TenantId, MediaBindingId), MediaReservation>,
}

/// In-memory unit of work that keeps pending writes separate until commit.
#[derive(Debug)]
pub struct InMemoryUnitOfWork {
    stores: Arc<Mutex<InMemoryStores>>,
    pending: Mutex<InMemoryStores>,
}

impl InMemoryUnitOfWork {
    /// Creates a new in-memory unit of work.
    pub fn new() -> Self {
        let stores = Arc::new(Mutex::new(InMemoryStores::default()));
        let pending = lock_mutex(&stores).clone();
        Self {
            stores,
            pending: Mutex::new(pending),
        }
    }

    /// Returns a snapshot of the committed stores.
    pub fn committed(&self) -> InMemoryStores {
        lock_mutex(&self.stores).clone()
    }

    /// Returns a snapshot of the pending stores.
    pub fn pending(&self) -> InMemoryStores {
        lock_mutex(&self.pending).clone()
    }

    fn with_pending<T>(&self, f: impl FnOnce(&mut InMemoryStores) -> T) -> T {
        let mut pending = lock_mutex(&self.pending);
        f(&mut pending)
    }
}

impl Default for InMemoryUnitOfWork {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl UnitOfWork for InMemoryUnitOfWork {
    fn device_repository(&mut self) -> &mut dyn DeviceRepository {
        self
    }

    fn channel_repository(&mut self) -> &mut dyn ChannelRepository {
        self
    }

    fn operation_repository(&mut self) -> &mut dyn OperationRepository {
        self
    }

    fn media_session_repository(&mut self) -> &mut dyn MediaSessionRepository {
        self
    }

    fn media_binding_repository(&mut self) -> &mut dyn MediaBindingRepository {
        self
    }

    fn outbox(&mut self) -> &mut dyn Outbox {
        self
    }

    async fn commit(&mut self) -> crate::Result<()> {
        let mut stores = lock_mutex(&self.stores);
        let pending = lock_mutex(&self.pending);
        *stores = pending.clone();
        Ok(())
    }

    async fn rollback(&mut self) -> crate::Result<()> {
        let stores = lock_mutex(&self.stores);
        let mut pending = lock_mutex(&self.pending);
        *pending = stores.clone();
        Ok(())
    }
}

#[async_trait::async_trait]
impl DeviceRepository for InMemoryUnitOfWork {
    async fn get(&self, tenant_id: TenantId, device_id: DeviceId) -> crate::Result<Option<Device>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending.devices.get(&(tenant_id, device_id)).cloned())
    }

    async fn get_by_external_id(
        &self,
        tenant_id: TenantId,
        protocol: crate::Protocol,
        external_id: ProtocolIdentity,
    ) -> crate::Result<Option<Device>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .devices
            .values()
            .find(|d| {
                d.tenant_id() == tenant_id
                    && d.protocol() == protocol
                    && d.external_id() == &external_id
            })
            .cloned())
    }

    async fn save(&self, device: &Device) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending
                .devices
                .insert((device.tenant_id(), device.device_id()), device.clone());
        });
        Ok(())
    }
}

#[async_trait::async_trait]
impl ChannelRepository for InMemoryUnitOfWork {
    async fn get(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
    ) -> crate::Result<Option<Channel>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .channels
            .get(&(tenant_id, device_id, channel_id))
            .cloned())
    }

    async fn list_by_device(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> crate::Result<Vec<Channel>> {
        let pending = lock_mutex(&self.pending);
        let mut channels: Vec<Channel> = pending
            .channels
            .values()
            .filter(|c| c.tenant_id() == tenant_id && c.device_id() == device_id)
            .cloned()
            .collect();
        channels.sort_by_key(|a| a.channel_id());
        Ok(channels)
    }

    async fn save(&self, channel: &Channel) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.channels.insert(
                (
                    channel.tenant_id(),
                    channel.device_id(),
                    channel.channel_id(),
                ),
                channel.clone(),
            );
        });
        Ok(())
    }

    async fn remove(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
    ) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.channels.remove(&(tenant_id, device_id, channel_id));
        });
        Ok(())
    }
}

#[async_trait::async_trait]
impl OperationRepository for InMemoryUnitOfWork {
    async fn get(
        &self,
        tenant_id: TenantId,
        operation_id: OperationId,
    ) -> crate::Result<Option<Operation>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending.operations.get(&(tenant_id, operation_id)).cloned())
    }

    async fn get_by_idempotency(
        &self,
        scope: &crate::IdempotencyScope,
    ) -> crate::Result<Option<Operation>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .operations
            .values()
            .find(|o| o.idempotency_scope() == scope)
            .cloned())
    }

    async fn save(&self, operation: &Operation) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.operations.insert(
                (operation.tenant_id(), operation.operation_id()),
                operation.clone(),
            );
        });
        Ok(())
    }
}

#[async_trait::async_trait]
impl MediaSessionRepository for InMemoryUnitOfWork {
    async fn get(
        &self,
        tenant_id: TenantId,
        media_session_id: MediaSessionId,
    ) -> crate::Result<Option<MediaSession>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .sessions
            .get(&(tenant_id, media_session_id))
            .cloned())
    }

    async fn get_by_idempotency(
        &self,
        scope: &crate::IdempotencyScope,
    ) -> crate::Result<Option<MediaSession>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .sessions
            .values()
            .find(|s| s.idempotency_scope() == scope)
            .cloned())
    }

    async fn save(&self, session: &MediaSession) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.sessions.insert(
                (session.tenant_id(), session.media_session_id()),
                session.clone(),
            );
        });
        Ok(())
    }
}

#[async_trait::async_trait]
impl MediaBindingRepository for InMemoryUnitOfWork {
    async fn get(
        &self,
        tenant_id: TenantId,
        media_binding_id: MediaBindingId,
    ) -> crate::Result<Option<MediaBinding>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .bindings
            .get(&(tenant_id, media_binding_id))
            .cloned())
    }

    async fn get_by_media_session(
        &self,
        tenant_id: TenantId,
        media_session_id: MediaSessionId,
    ) -> crate::Result<Option<MediaBinding>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .bindings
            .values()
            .find(|b| b.tenant_id() == tenant_id && b.media_session_id() == media_session_id)
            .cloned())
    }

    async fn save(&self, binding: &MediaBinding) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.bindings.insert(
                (binding.tenant_id(), binding.media_binding_id()),
                binding.clone(),
            );
        });
        Ok(())
    }
}

#[async_trait::async_trait]
impl Outbox for InMemoryUnitOfWork {
    async fn append(&self, event: Event<DomainEvent>) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.outbox.push(OutboxEntry {
                event,
                published: false,
            });
        });
        Ok(())
    }

    async fn pending(&self, limit: usize) -> crate::Result<Vec<OutboxEntry>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .outbox
            .iter()
            .filter(|e| !e.published)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn mark_published(&self, event_id: cheetah_signal_types::EventId) -> crate::Result<()> {
        self.with_pending(|pending| {
            for entry in &mut pending.outbox {
                if entry.event.event_id == event_id {
                    entry.published = true;
                    break;
                }
            }
        });
        Ok(())
    }
}

/// In-memory outbox that operates directly on committed stores.
#[derive(Clone, Debug, Default)]
pub struct InMemoryOutbox {
    stores: Arc<Mutex<InMemoryStores>>,
}

impl InMemoryOutbox {
    /// Creates a new in-memory outbox.
    pub fn new(stores: Arc<Mutex<InMemoryStores>>) -> Self {
        Self { stores }
    }
}

#[async_trait::async_trait]
impl Outbox for InMemoryOutbox {
    async fn append(&self, event: Event<DomainEvent>) -> crate::Result<()> {
        let mut stores = lock_mutex(&self.stores);
        stores.outbox.push(OutboxEntry {
            event,
            published: false,
        });
        Ok(())
    }

    async fn pending(&self, limit: usize) -> crate::Result<Vec<OutboxEntry>> {
        let stores = lock_mutex(&self.stores);
        Ok(stores
            .outbox
            .iter()
            .filter(|e| !e.published)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn mark_published(&self, event_id: cheetah_signal_types::EventId) -> crate::Result<()> {
        let mut stores = lock_mutex(&self.stores);
        for entry in &mut stores.outbox {
            if entry.event.event_id == event_id {
                entry.published = true;
                break;
            }
        }
        Ok(())
    }
}

/// In-memory command bus that stores dispatched commands.
#[derive(Clone, Debug, Default)]
pub struct InMemoryCommandBus {
    commands: Arc<Mutex<Vec<Command>>>,
}

impl InMemoryCommandBus {
    /// Creates a new in-memory command bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of all dispatched commands.
    pub fn snapshot(&self) -> Vec<Command> {
        lock_mutex(&self.commands).clone()
    }
}

#[async_trait::async_trait]
impl CommandBus for InMemoryCommandBus {
    async fn send(&self, command: &Command) -> crate::Result<()> {
        lock_mutex(&self.commands).push(command.clone());
        Ok(())
    }
}

/// In-memory device owner resolver.
#[derive(Clone, Debug, Default)]
pub struct InMemoryDeviceOwnerResolver {
    owners: Arc<Mutex<BTreeMap<(TenantId, DeviceId), OwnerInfo>>>,
}

impl InMemoryDeviceOwnerResolver {
    /// Creates a new resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the owner for a device.
    pub fn set_owner(&self, tenant_id: TenantId, device_id: DeviceId, owner: OwnerInfo) {
        lock_mutex(&self.owners).insert((tenant_id, device_id), owner);
    }
}

#[async_trait::async_trait]
impl crate::DeviceOwnerResolver for InMemoryDeviceOwnerResolver {
    async fn resolve(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> crate::Result<Option<OwnerInfo>> {
        let owners = lock_mutex(&self.owners);
        Ok(owners.get(&(tenant_id, device_id)).cloned())
    }
}

/// In-memory media port.
#[derive(Clone)]
pub struct InMemoryMediaPort {
    reservations: Arc<Mutex<BTreeMap<(TenantId, MediaBindingId), MediaReservation>>>,
    id_generator: Arc<dyn IdGenerator>,
}

impl std::fmt::Debug for InMemoryMediaPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryMediaPort")
            .field("reservations", &self.reservations)
            .finish_non_exhaustive()
    }
}

impl InMemoryMediaPort {
    /// Creates a new in-memory media port.
    pub fn new(id_generator: Arc<dyn IdGenerator>) -> Self {
        Self {
            reservations: Arc::new(Mutex::new(BTreeMap::new())),
            id_generator,
        }
    }

    fn reserve(
        &self,
        tenant_id: TenantId,
        media_binding_id: MediaBindingId,
    ) -> crate::Result<MediaReservation> {
        let mut reservations = lock_mutex(&self.reservations);
        let key = (tenant_id, media_binding_id);
        if reservations.contains_key(&key) {
            return Err(DomainError::unavailable("media binding already reserved"));
        }
        let reservation = MediaReservation {
            media_node_id: self.id_generator.generate_node_id(),
            media_node_instance_epoch: self.id_generator.generate_media_node_instance_epoch(),
        };
        reservations.insert(key, reservation.clone());
        Ok(reservation)
    }
}

#[async_trait::async_trait]
impl crate::MediaPort for InMemoryMediaPort {
    async fn reserve_live(
        &self,
        tenant_id: TenantId,
        _device_id: DeviceId,
        _channel_id: ChannelId,
        _media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        _purpose: crate::MediaPurpose,
    ) -> crate::Result<MediaReservation> {
        self.reserve(tenant_id, media_binding_id)
    }

    async fn reserve_playback(
        &self,
        tenant_id: TenantId,
        _device_id: DeviceId,
        _channel_id: ChannelId,
        _media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        _start_time: UtcTimestamp,
        _end_time: UtcTimestamp,
        _scale: f64,
    ) -> crate::Result<MediaReservation> {
        self.reserve(tenant_id, media_binding_id)
    }

    async fn reserve_talk(
        &self,
        tenant_id: TenantId,
        _device_id: DeviceId,
        _channel_id: ChannelId,
        _media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
    ) -> crate::Result<MediaReservation> {
        self.reserve(tenant_id, media_binding_id)
    }

    async fn release(
        &self,
        tenant_id: TenantId,
        media_binding_id: MediaBindingId,
    ) -> crate::Result<()> {
        lock_mutex(&self.reservations).remove(&(tenant_id, media_binding_id));
        Ok(())
    }
}

/// In-memory event publisher.
#[derive(Clone, Debug, Default)]
pub struct InMemoryEventPublisher {
    events: Arc<Mutex<Vec<Event<DomainEvent>>>>,
}

impl InMemoryEventPublisher {
    /// Creates a new in-memory event publisher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of all published events.
    pub fn snapshot(&self) -> Vec<Event<DomainEvent>> {
        lock_mutex(&self.events).clone()
    }
}

#[async_trait::async_trait]
impl EventPublisher for InMemoryEventPublisher {
    async fn publish(&self, event: &Event<DomainEvent>) -> crate::Result<()> {
        lock_mutex(&self.events).push(event.clone());
        Ok(())
    }
}

/// Creates a minimal request context for tests.
pub fn request_context(
    tenant_id: TenantId,
    id_generator: &dyn IdGenerator,
    _clock: &dyn Clock,
) -> RequestContext {
    RequestContext {
        tenant_id,
        principal: Principal {
            id: "test".to_string(),
            kind: cheetah_signal_types::PrincipalKind::Service,
            scopes: vec![],
        },
        message_id: id_generator.generate_message_id(),
        correlation_id: id_generator.generate_correlation_id(),
        traceparent: None,
        tracestate: None,
        deadline: None,
        node_id: Some(id_generator.generate_node_id()),
    }
}

/// Helper to build a `ResourceRef` for a device.
pub fn device_resource_ref(tenant_id: TenantId, device_id: DeviceId) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    }
}

/// Helper to build a `ResourceRef` for a media session.
pub fn media_session_resource_ref(
    tenant_id: TenantId,
    media_session_id: MediaSessionId,
) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::MediaSession,
        id: ResourceId::MediaSession(media_session_id),
    }
}

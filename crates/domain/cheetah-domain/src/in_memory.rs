//! In-memory test helpers for `cheetah-domain`.
//!
//! These implementations are deterministic and safe to use from synchronous or
//! asynchronous tests. They use `std::sync::Mutex` and never hold a lock across
//! an `.await` point.

use crate::GbPlatformLink;
use crate::{
    Channel, ChannelRepository, ChannelStatus, Command, CommandBus, CommandPayload, DeliveryStatus,
    Device, DeviceLifecycle, DeviceRepository, DomainError, DomainEvent, EventPublisher,
    MediaBinding, MediaBindingRepository, MediaNode, MediaNodeCommand, MediaNodeCommandResult,
    MediaNodeSessionRef, MediaPurpose, MediaReservation, MediaSession, MediaSessionRepository,
    MediaSessionState, Operation, OperationRepository, OperationStatus, Outbox, OutboxEntry,
    OwnerInfo, PlatformDirection, PlatformLinkRepository, ProcessedMessageRecord,
    ProcessedMessageRepository, ProcessedMessageStatus, Protocol, ProtocolSession,
    ProtocolSessionRepository, UnitOfWork, WebhookConfig, WebhookConfigRepository, WebhookDelivery,
    WebhookDeliveryRepository,
};
use cheetah_signal_types::{
    ChannelId, Clock, DeliveryId, DeviceId, DurationMs, Event, IdGenerator, ListCursor,
    MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, OperationId,
    OwnerEpoch, Page, PageRequest, PlatformLinkId, Principal, ProtocolIdentity, ProtocolSessionId,
    RequestContext, ResourceId, ResourceKind, ResourceRef, Revision, TenantId, UtcTimestamp,
    WebhookId,
};
use std::collections::{BTreeMap, VecDeque};
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

/// Inserts `value` into `map`. A fresh insert (absent key) is always accepted,
/// mirroring the SQL upsert whose `WHERE revision = EXCLUDED.revision - 1` guard
/// is only evaluated on conflict. When the key already exists, the insert is
/// accepted only if the existing entry's revision is exactly one less than
/// `value`'s revision, mirroring the SQL optimistic concurrency guard.
fn save_with_revision<K, V>(
    map: &mut BTreeMap<K, V>,
    key: K,
    value: V,
    revision: impl Fn(&V) -> Revision,
) -> crate::Result<()>
where
    K: Ord + Clone,
{
    let value_revision = revision(&value).0;
    match map.get(&key) {
        Some(existing) if value_revision == 0 => Err(DomainError::ConcurrentModification {
            expected: 0,
            found: revision(existing).0,
        }),
        Some(existing) if revision(existing).0 == value_revision - 1 => {
            map.insert(key, value);
            Ok(())
        }
        Some(existing) => Err(DomainError::ConcurrentModification {
            expected: value_revision - 1,
            found: revision(existing).0,
        }),
        None => {
            map.insert(key, value);
            Ok(())
        }
    }
}

/// Decodes an opaque list cursor into the UUID it represents.
fn decode_cursor(cursor: &Option<String>) -> crate::Result<Option<uuid::Uuid>> {
    match cursor {
        None => Ok(None),
        Some(value) => {
            let c = ListCursor::decode(value)
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            let (_ts, id) = c
                .parse()
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            Ok(Some(id))
        }
    }
}

/// Slices the sorted items into a page, encoding the next cursor when present.
fn to_page<T>(
    items: Vec<T>,
    page: PageRequest,
    id_of: fn(&T) -> uuid::Uuid,
) -> crate::Result<Page<T>> {
    let page_size = page.page_size as usize;
    let next_cursor = if items.len() > page_size {
        let last = &items[page_size - 1];
        // In-memory timestamps are not part of the cursor; use a fixed epoch for encoding.
        let ts = UtcTimestamp::from_offset(OffsetDateTime::UNIX_EPOCH);
        Some(
            ListCursor::new(ts, id_of(last))
                .map_err(|e| DomainError::internal(format!("invalid cursor: {e}")))?
                .encode()
                .map_err(|e| DomainError::internal(format!("failed to encode cursor: {e}")))?,
        )
    } else {
        None
    };
    let mut page = Page::new(items.into_iter().take(page_size).collect());
    if let Some(cursor) = next_cursor {
        page = page.with_next_cursor(cursor);
    }
    Ok(page)
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

    fn generate_platform_link_id(&self) -> cheetah_signal_types::PlatformLinkId {
        cheetah_signal_types::PlatformLinkId::from_uuid(self.next_uuid())
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

    fn generate_node_instance_id(&self) -> cheetah_signal_types::NodeInstanceId {
        cheetah_signal_types::NodeInstanceId::from_uuid(self.next_uuid())
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

    fn generate_webhook_id(&self) -> cheetah_signal_types::WebhookId {
        cheetah_signal_types::WebhookId::from_uuid(self.next_uuid())
    }

    fn generate_delivery_id(&self) -> cheetah_signal_types::DeliveryId {
        cheetah_signal_types::DeliveryId::from_uuid(self.next_uuid())
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
    /// Processed messages keyed by `(tenant_id, message_id)`.
    pub processed_messages: BTreeMap<(TenantId, MessageId), ProcessedMessageRecord>,
    /// Webhook configurations keyed by `(tenant_id, webhook_id)`.
    pub webhook_configs: BTreeMap<(TenantId, WebhookId), WebhookConfig>,
    /// Webhook deliveries keyed by `(tenant_id, delivery_id)`.
    pub webhook_deliveries: BTreeMap<(TenantId, DeliveryId), WebhookDelivery>,
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

    fn processed_message_repository(&mut self) -> &mut dyn ProcessedMessageRepository {
        self
    }

    fn webhook_config_repository(&mut self) -> &mut dyn WebhookConfigRepository {
        self
    }

    fn webhook_delivery_repository(&mut self) -> &mut dyn WebhookDeliveryRepository {
        self
    }

    fn outbox(&mut self) -> &mut dyn Outbox {
        self
    }

    async fn acquire_ownership(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        now: UtcTimestamp,
        lease_until: UtcTimestamp,
    ) -> crate::Result<Option<(OwnerInfo, Option<OwnerInfo>)>> {
        let previous =
            self.with_pending(|pending| pending.owners.get(&(tenant_id, device_id)).cloned());
        let can_take = match &previous {
            None => true,
            Some(owner) => {
                owner.lease_until.is_some_and(|lease| lease <= now)
                    || owner.owner_node_id == node_id
            }
        };
        if !can_take {
            return Ok(None);
        }
        let epoch = previous.as_ref().map_or(1, |o| o.owner_epoch.0 + 1);
        let owner = OwnerInfo {
            owner_node_id: node_id,
            owner_epoch: OwnerEpoch(epoch),
            lease_until: Some(lease_until),
        };
        self.with_pending(|pending| {
            pending.owners.insert((tenant_id, device_id), owner.clone());
        });
        Ok(Some((owner, previous)))
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
    async fn get(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> crate::Result<Option<Device>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .devices
            .get(&(tenant_id, device_id))
            .filter(|d| d.lifecycle() != DeviceLifecycle::Retired)
            .cloned())
    }

    async fn get_by_external_id(
        &mut self,
        tenant_id: TenantId,
        protocol: crate::Protocol,
        external_id: ProtocolIdentity,
    ) -> crate::Result<Option<Device>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .devices
            .values()
            .find(|d| {
                d.lifecycle() != DeviceLifecycle::Retired
                    && d.tenant_id() == tenant_id
                    && d.protocol() == protocol
                    && d.external_id() == &external_id
            })
            .cloned())
    }

    async fn save(&mut self, device: &Device) -> crate::Result<()> {
        self.with_pending(|pending| {
            save_with_revision(
                &mut pending.devices,
                (device.tenant_id(), device.device_id()),
                device.clone(),
                Device::revision,
            )
        })
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        protocol: Option<String>,
        lifecycle: Option<String>,
        name_prefix: Option<String>,
        updated_after: Option<UtcTimestamp>,
        page: PageRequest,
    ) -> crate::Result<Page<Device>> {
        let cursor = decode_cursor(&page.cursor)?;
        let pending = lock_mutex(&self.pending);
        let mut items: Vec<Device> = pending
            .devices
            .values()
            .filter(|d| {
                d.lifecycle() != DeviceLifecycle::Retired
                    && d.tenant_id() == tenant_id
                    && protocol
                        .as_ref()
                        .is_none_or(|p| d.protocol().to_string() == *p)
                    && lifecycle
                        .as_ref()
                        .is_none_or(|l| d.lifecycle().to_string() == *l)
                    && name_prefix.as_ref().is_none_or(|p| d.name().starts_with(p))
                    && updated_after.is_none_or(|t| d.updated_at() > t)
                    && cursor.is_none_or(|c| d.device_id().as_uuid() > c)
            })
            .cloned()
            .collect();
        items.sort_by_key(|d| d.device_id().as_uuid());
        drop(pending);
        to_page(items, page, |d| d.device_id().as_uuid())
    }
}

#[async_trait::async_trait]
impl ChannelRepository for InMemoryUnitOfWork {
    async fn get(
        &mut self,
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
        &mut self,
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

    async fn save(&mut self, channel: &Channel) -> crate::Result<()> {
        self.with_pending(|pending| {
            save_with_revision(
                &mut pending.channels,
                (
                    channel.tenant_id(),
                    channel.device_id(),
                    channel.channel_id(),
                ),
                channel.clone(),
                Channel::revision,
            )
        })
    }

    async fn remove(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
        expected_revision: Revision,
    ) -> crate::Result<()> {
        let key = (tenant_id, device_id, channel_id);
        self.with_pending(|pending| match pending.channels.get(&key) {
            Some(existing) if existing.revision() == expected_revision => {
                pending.channels.remove(&key);
                Ok(())
            }
            Some(existing) => Err(DomainError::ConcurrentModification {
                expected: expected_revision.0,
                found: existing.revision().0,
            }),
            None => Err(DomainError::ConcurrentModification {
                expected: expected_revision.0,
                found: 0,
            }),
        })
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        status: Option<String>,
        name_prefix: Option<String>,
        updated_after: Option<UtcTimestamp>,
        page: PageRequest,
    ) -> crate::Result<Page<Channel>> {
        let cursor = decode_cursor(&page.cursor)?;
        let pending = lock_mutex(&self.pending);
        let mut items: Vec<Channel> = pending
            .channels
            .values()
            .filter(|c| {
                c.tenant_id() == tenant_id
                    && c.device_id() == device_id
                    && status.as_ref().is_none_or(|s| {
                        c.status().to_string() == *s
                            || s.parse::<ChannelStatus>().is_ok_and(|x| x == c.status())
                    })
                    && name_prefix.as_ref().is_none_or(|p| c.name().starts_with(p))
                    && updated_after.is_none_or(|t| c.updated_at() > t)
                    && cursor.is_none_or(|cur| c.channel_id().as_uuid() > cur)
            })
            .cloned()
            .collect();
        items.sort_by_key(|c| c.channel_id().as_uuid());
        drop(pending);
        to_page(items, page, |c| c.channel_id().as_uuid())
    }
}

#[async_trait::async_trait]
impl OperationRepository for InMemoryUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: TenantId,
        operation_id: OperationId,
    ) -> crate::Result<Option<Operation>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending.operations.get(&(tenant_id, operation_id)).cloned())
    }

    async fn get_by_idempotency(
        &mut self,
        scope: &crate::IdempotencyScope,
    ) -> crate::Result<Option<Operation>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .operations
            .values()
            .find(|o| o.idempotency_scope() == scope)
            .cloned())
    }

    async fn save(&mut self, operation: &Operation) -> crate::Result<()> {
        self.with_pending(|pending| {
            save_with_revision(
                &mut pending.operations,
                (operation.tenant_id(), operation.operation_id()),
                operation.clone(),
                Operation::revision,
            )
        })
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        device_id: Option<DeviceId>,
        status: Option<String>,
        updated_after: Option<UtcTimestamp>,
        page: PageRequest,
    ) -> crate::Result<Page<Operation>> {
        let cursor = decode_cursor(&page.cursor)?;
        let pending = lock_mutex(&self.pending);
        let mut items: Vec<Operation> = pending
            .operations
            .values()
            .filter(|o| {
                o.tenant_id() == tenant_id
                    && device_id.is_none_or(|d| d == o.device_id())
                    && status.as_ref().is_none_or(|s| {
                        o.status().to_string() == *s
                            || s.parse::<OperationStatus>().is_ok_and(|x| x == o.status())
                    })
                    && updated_after.is_none_or(|t| o.updated_at() > t)
                    && cursor.is_none_or(|cur| o.operation_id().as_uuid() > cur)
            })
            .cloned()
            .collect();
        items.sort_by_key(|o| o.operation_id().as_uuid());
        drop(pending);
        to_page(items, page, |o| o.operation_id().as_uuid())
    }
}

#[async_trait::async_trait]
impl MediaSessionRepository for InMemoryUnitOfWork {
    async fn get(
        &mut self,
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
        &mut self,
        scope: &crate::IdempotencyScope,
    ) -> crate::Result<Option<MediaSession>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .sessions
            .values()
            .find(|s| s.idempotency_scope() == scope)
            .cloned())
    }

    async fn save(&mut self, session: &MediaSession) -> crate::Result<()> {
        self.with_pending(|pending| {
            save_with_revision(
                &mut pending.sessions,
                (session.tenant_id(), session.media_session_id()),
                session.clone(),
                MediaSession::revision,
            )
        })
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        device_id: Option<DeviceId>,
        purpose: Option<String>,
        state: Option<String>,
        updated_after: Option<UtcTimestamp>,
        page: PageRequest,
    ) -> crate::Result<Page<MediaSession>> {
        let cursor = decode_cursor(&page.cursor)?;
        let pending = lock_mutex(&self.pending);
        let mut items: Vec<MediaSession> = pending
            .sessions
            .values()
            .filter(|s| {
                s.tenant_id() == tenant_id
                    && device_id.is_none_or(|d| d == s.device_id())
                    && purpose.as_ref().is_none_or(|p| {
                        s.purpose().to_string() == *p
                            || p.parse::<MediaPurpose>().is_ok_and(|x| x == s.purpose())
                    })
                    && state.as_ref().is_none_or(|st| {
                        s.state().to_string() == *st
                            || st
                                .parse::<MediaSessionState>()
                                .is_ok_and(|x| x == s.state())
                    })
                    && updated_after.is_none_or(|t| s.updated_at() > t)
                    && cursor.is_none_or(|cur| s.media_session_id().as_uuid() > cur)
            })
            .cloned()
            .collect();
        items.sort_by_key(|s| s.media_session_id().as_uuid());
        drop(pending);
        to_page(items, page, |s| s.media_session_id().as_uuid())
    }
}

#[async_trait::async_trait]
impl MediaBindingRepository for InMemoryUnitOfWork {
    async fn get(
        &mut self,
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
        &mut self,
        tenant_id: TenantId,
        media_session_id: MediaSessionId,
    ) -> crate::Result<Option<MediaBinding>> {
        let pending = lock_mutex(&self.pending);
        let mut candidates: Vec<&MediaBinding> = pending
            .bindings
            .values()
            .filter(|b| b.tenant_id() == tenant_id && b.media_session_id() == media_session_id)
            .collect();
        // Prefer the most recent non-terminal binding, matching the SQL
        // repository contract that returns `None` when all bindings are terminal.
        candidates.sort_by_key(|b| b.created_at());
        let active = candidates.iter().rev().find(|b| !b.is_terminal()).copied();
        Ok(active.cloned())
    }

    async fn save(&mut self, binding: &MediaBinding) -> crate::Result<()> {
        self.with_pending(|pending| {
            save_with_revision(
                &mut pending.bindings,
                (binding.tenant_id(), binding.media_binding_id()),
                binding.clone(),
                MediaBinding::revision,
            )
        })
    }
}

#[async_trait::async_trait]
impl ProcessedMessageRepository for InMemoryUnitOfWork {
    async fn find(
        &mut self,
        tenant_id: TenantId,
        message_id: MessageId,
    ) -> crate::Result<Option<ProcessedMessageRecord>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .processed_messages
            .get(&(tenant_id, message_id))
            .cloned())
    }

    async fn get_or_insert(
        &mut self,
        record: ProcessedMessageRecord,
    ) -> crate::Result<Option<ProcessedMessageRecord>> {
        if let Some(existing) = self.find(record.tenant_id, record.message_id).await? {
            return Ok(Some(existing));
        }
        let mut pending = lock_mutex(&self.pending);
        pending
            .processed_messages
            .insert((record.tenant_id, record.message_id), record);
        Ok(None)
    }

    async fn complete(
        &mut self,
        tenant_id: TenantId,
        message_id: MessageId,
        status: ProcessedMessageStatus,
        result_payload: Option<String>,
        processed_at: UtcTimestamp,
    ) -> crate::Result<()> {
        let mut pending = lock_mutex(&self.pending);
        if let Some(entry) = pending.processed_messages.get_mut(&(tenant_id, message_id)) {
            entry.status = status;
            entry.result_payload = result_payload;
            entry.processed_at = processed_at;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl WebhookConfigRepository for InMemoryUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: TenantId,
        webhook_id: WebhookId,
    ) -> crate::Result<Option<WebhookConfig>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .webhook_configs
            .get(&(tenant_id, webhook_id))
            .cloned())
    }

    async fn save(&mut self, config: &WebhookConfig) -> crate::Result<()> {
        self.with_pending(|pending| {
            save_with_revision(
                &mut pending.webhook_configs,
                (config.tenant_id(), config.webhook_id()),
                config.clone(),
                WebhookConfig::revision,
            )
        })
    }

    async fn delete(&mut self, tenant_id: TenantId, webhook_id: WebhookId) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.webhook_configs.remove(&(tenant_id, webhook_id));
        });
        Ok(())
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        enabled: Option<bool>,
        event_type: Option<String>,
        page: PageRequest,
    ) -> crate::Result<Page<WebhookConfig>> {
        let cursor = decode_cursor(&page.cursor)?;
        let pending = lock_mutex(&self.pending);
        let mut items: Vec<WebhookConfig> = pending
            .webhook_configs
            .values()
            .filter(|c| {
                c.tenant_id() == tenant_id
                    && enabled.is_none_or(|e| c.enabled() == e)
                    && event_type
                        .as_ref()
                        .is_none_or(|t| c.event_types().is_empty() || c.event_types().contains(t))
                    && cursor.is_none_or(|cur| c.webhook_id().as_uuid() > cur)
            })
            .cloned()
            .collect();
        items.sort_by_key(|c| c.webhook_id().as_uuid());
        drop(pending);
        to_page(items, page, |c| c.webhook_id().as_uuid())
    }
}

#[async_trait::async_trait]
impl WebhookDeliveryRepository for InMemoryUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: TenantId,
        delivery_id: DeliveryId,
    ) -> crate::Result<Option<WebhookDelivery>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .webhook_deliveries
            .get(&(tenant_id, delivery_id))
            .cloned())
    }

    async fn save(&mut self, delivery: &WebhookDelivery) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.webhook_deliveries.insert(
                (delivery.tenant_id(), delivery.delivery_id()),
                delivery.clone(),
            );
        });
        Ok(())
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        webhook_id: WebhookId,
        status: Option<String>,
        page: PageRequest,
    ) -> crate::Result<Page<WebhookDelivery>> {
        let cursor = decode_cursor(&page.cursor)?;
        let pending = lock_mutex(&self.pending);
        let mut items: Vec<WebhookDelivery> = pending
            .webhook_deliveries
            .values()
            .filter(|d| {
                d.tenant_id() == tenant_id
                    && d.webhook_id() == webhook_id
                    && status.as_ref().is_none_or(|s| {
                        d.status().to_string() == *s
                            || s.parse::<DeliveryStatus>().is_ok_and(|x| x == d.status())
                    })
                    && cursor.is_none_or(|cur| d.delivery_id().as_uuid() > cur)
            })
            .cloned()
            .collect();
        items.sort_by_key(|d| d.delivery_id().as_uuid());
        drop(pending);
        to_page(items, page, |d| d.delivery_id().as_uuid())
    }

    async fn pending(
        &mut self,
        now: UtcTimestamp,
        limit: usize,
    ) -> crate::Result<Vec<WebhookDelivery>> {
        let pending = lock_mutex(&self.pending);
        let mut items: Vec<WebhookDelivery> = pending
            .webhook_deliveries
            .values()
            .filter(|d| {
                (d.status() == DeliveryStatus::Pending || d.status() == DeliveryStatus::Failed)
                    && d.next_attempt_at().is_none_or(|t| t <= now)
            })
            .take(limit)
            .cloned()
            .collect();
        drop(pending);
        items.sort_by_key(|a| a.next_attempt_at());
        Ok(items)
    }
}

#[async_trait::async_trait]
impl Outbox for InMemoryUnitOfWork {
    async fn append(&mut self, event: Event<DomainEvent>) -> crate::Result<()> {
        self.with_pending(|pending| {
            pending.outbox.push(OutboxEntry {
                event,
                published: false,
                attempts: 0,
                failed: false,
                error: None,
                next_attempt_at: None,
            });
        });
        Ok(())
    }

    async fn pending(
        &mut self,
        now: UtcTimestamp,
        limit: usize,
    ) -> crate::Result<Vec<OutboxEntry>> {
        let pending = lock_mutex(&self.pending);
        Ok(pending
            .outbox
            .iter()
            .filter(|e| !e.published && !e.failed && e.next_attempt_at.is_none_or(|t| t <= now))
            .take(limit)
            .cloned()
            .collect())
    }

    async fn mark_published(
        &mut self,
        event_id: cheetah_signal_types::EventId,
    ) -> crate::Result<()> {
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

    async fn mark_failed(
        &mut self,
        event_id: cheetah_signal_types::EventId,
        attempts: u32,
        failed: bool,
        error: Option<String>,
        next_attempt_at: Option<UtcTimestamp>,
    ) -> crate::Result<()> {
        self.with_pending(|pending| {
            for entry in &mut pending.outbox {
                if entry.event.event_id == event_id {
                    entry.attempts = attempts;
                    entry.failed = failed;
                    entry.error.clone_from(&error);
                    entry.next_attempt_at = next_attempt_at;
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
    async fn append(&mut self, event: Event<DomainEvent>) -> crate::Result<()> {
        let mut stores = lock_mutex(&self.stores);
        stores.outbox.push(OutboxEntry {
            event,
            published: false,
            attempts: 0,
            failed: false,
            error: None,
            next_attempt_at: None,
        });
        Ok(())
    }

    async fn pending(
        &mut self,
        now: UtcTimestamp,
        limit: usize,
    ) -> crate::Result<Vec<OutboxEntry>> {
        let stores = lock_mutex(&self.stores);
        Ok(stores
            .outbox
            .iter()
            .filter(|e| !e.published && !e.failed && e.next_attempt_at.is_none_or(|t| t <= now))
            .take(limit)
            .cloned()
            .collect())
    }

    async fn mark_published(
        &mut self,
        event_id: cheetah_signal_types::EventId,
    ) -> crate::Result<()> {
        let mut stores = lock_mutex(&self.stores);
        for entry in &mut stores.outbox {
            if entry.event.event_id == event_id {
                entry.published = true;
                break;
            }
        }
        Ok(())
    }

    async fn mark_failed(
        &mut self,
        event_id: cheetah_signal_types::EventId,
        attempts: u32,
        failed: bool,
        error: Option<String>,
        next_attempt_at: Option<UtcTimestamp>,
    ) -> crate::Result<()> {
        let mut stores = lock_mutex(&self.stores);
        for entry in &mut stores.outbox {
            if entry.event.event_id == event_id {
                entry.attempts = attempts;
                entry.failed = failed;
                entry.error.clone_from(&error);
                entry.next_attempt_at = next_attempt_at;
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

type MediaNodeSessionMap = BTreeMap<(TenantId, NodeId), Vec<MediaNodeSessionRef>>;

type MediaNodeMap = BTreeMap<(TenantId, NodeId), MediaNode>;

/// In-memory media port.
#[derive(Clone)]
pub struct InMemoryMediaPort {
    reservations: Arc<Mutex<BTreeMap<(TenantId, MediaBindingId), MediaReservation>>>,
    node_sessions: Arc<Mutex<MediaNodeSessionMap>>,
    id_generator: Arc<dyn IdGenerator>,
    /// Scripted results consumed one-per-call by [`InMemoryMediaPort::execute`],
    /// used to deterministically drive failure/unknown-outcome contract tests.
    scripted_execute: Arc<Mutex<VecDeque<MediaNodeCommandResult>>>,
    /// Scripted reservation errors consumed one-per-call by the `reserve_*`
    /// methods, used to exercise reservation failure and compensation paths.
    scripted_reserve_errors: Arc<Mutex<VecDeque<DomainError>>>,
    /// Scripted media nodes returned by [`InMemoryMediaPort::get_node`] and
    /// included in [`InMemoryMediaPort::list_nodes`]. Used to exercise reconcile
    /// paths that depend on node status, health and lease state.
    scripted_nodes: Arc<Mutex<MediaNodeMap>>,
}

impl std::fmt::Debug for InMemoryMediaPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryMediaPort")
            .field("reservations", &self.reservations)
            .field("node_sessions", &self.node_sessions)
            .finish_non_exhaustive()
    }
}

impl InMemoryMediaPort {
    /// Creates a new in-memory media port.
    pub fn new(id_generator: Arc<dyn IdGenerator>) -> Self {
        Self {
            reservations: Arc::new(Mutex::new(BTreeMap::new())),
            node_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            id_generator,
            scripted_execute: Arc::new(Mutex::new(VecDeque::new())),
            scripted_reserve_errors: Arc::new(Mutex::new(VecDeque::new())),
            scripted_nodes: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Queues a result to be returned by the next [`InMemoryMediaPort::execute`]
    /// call instead of the default behavior. Results are consumed in FIFO order;
    /// once the queue is empty the default behavior resumes.
    pub fn script_execute_result(&self, result: MediaNodeCommandResult) {
        lock_mutex(&self.scripted_execute).push_back(result);
    }

    /// Queues an error to be returned by the next `reserve_*` call.
    pub fn script_reserve_error(&self, error: DomainError) {
        lock_mutex(&self.scripted_reserve_errors).push_back(error);
    }

    /// Configures the media node metadata returned by `get_node` and included in
    /// `list_nodes` for the given tenant. This lets tests drive reconcile paths
    /// that depend on node status, health and lease state without a real
    /// scheduler registry.
    pub fn set_node(&self, tenant_id: TenantId, node: MediaNode) {
        lock_mutex(&self.scripted_nodes).insert((tenant_id, node.node_id), node);
    }

    /// Removes a scripted media node for the given tenant.
    pub fn remove_node(&self, tenant_id: TenantId, node_id: NodeId) {
        lock_mutex(&self.scripted_nodes).remove(&(tenant_id, node_id));
    }

    /// Clears all scripted media nodes.
    pub fn clear_nodes(&self) {
        lock_mutex(&self.scripted_nodes).clear();
    }

    /// Configures the sessions reported by a media node for reconciliation tests.
    /// Passing an empty vector removes the node so it no longer appears in
    /// `list_nodes`, which is useful for simulating a deregistered or expired
    /// media node.
    pub fn set_node_sessions(
        &self,
        tenant_id: TenantId,
        node_id: NodeId,
        sessions: Vec<MediaNodeSessionRef>,
    ) {
        let mut map = lock_mutex(&self.node_sessions);
        if sessions.is_empty() {
            map.remove(&(tenant_id, node_id));
        } else {
            map.insert((tenant_id, node_id), sessions);
        }
    }

    fn reserve(
        &self,
        tenant_id: TenantId,
        media_binding_id: MediaBindingId,
    ) -> crate::Result<MediaReservation> {
        if let Some(error) = lock_mutex(&self.scripted_reserve_errors).pop_front() {
            return Err(error);
        }
        let mut reservations = lock_mutex(&self.reservations);
        let key = (tenant_id, media_binding_id);
        if reservations.contains_key(&key) {
            return Err(DomainError::unavailable("media binding already reserved"));
        }
        let reservation = MediaReservation {
            media_node_id: self.id_generator.generate_node_id(),
            media_node_instance_epoch: self.id_generator.generate_media_node_instance_epoch(),
            contract_version: 1,
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
        _requirements: &crate::MediaRequirements,
        _clock: &dyn Clock,
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
        _requirements: &crate::MediaRequirements,
        _clock: &dyn Clock,
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
        _requirements: &crate::MediaRequirements,
        _clock: &dyn Clock,
    ) -> crate::Result<MediaReservation> {
        self.reserve(tenant_id, media_binding_id)
    }

    async fn release(
        &self,
        tenant_id: TenantId,
        media_binding_id: MediaBindingId,
        _clock: &dyn Clock,
    ) -> crate::Result<()> {
        lock_mutex(&self.reservations).remove(&(tenant_id, media_binding_id));
        Ok(())
    }

    async fn execute(
        &self,
        command: MediaNodeCommand,
        _clock: &dyn Clock,
    ) -> crate::Result<MediaNodeCommandResult> {
        if let Some(result) = lock_mutex(&self.scripted_execute).pop_front() {
            return Ok(result);
        }
        let tenant_id = command.tenant_id;
        let node_id = command.media_node_id;
        let epoch = command.media_node_instance_epoch;
        match command.payload {
            CommandPayload::StartLive {
                media_session_id,
                channel_id,
                ..
            }
            | CommandPayload::StartPlayback {
                media_session_id,
                channel_id,
                ..
            }
            | CommandPayload::StartTalk {
                media_session_id,
                channel_id,
                ..
            }
            | CommandPayload::StartBroadcast {
                media_session_id,
                channel_id,
                ..
            } => {
                let mut sessions = lock_mutex(&self.node_sessions);
                let refs = sessions.entry((tenant_id, node_id)).or_default();
                refs.retain(|r| r.media_session_id != media_session_id);
                refs.push(MediaNodeSessionRef {
                    media_session_id,
                    device_id: None,
                    channel_id: Some(channel_id),
                    media_node_instance_epoch: epoch,
                });
                Ok(MediaNodeCommandResult::Accepted)
            }
            CommandPayload::StopMediaSession {
                media_session_id, ..
            } => {
                let mut sessions = lock_mutex(&self.node_sessions);
                if let Some(refs) = sessions.get_mut(&(tenant_id, node_id)) {
                    refs.retain(|r| r.media_session_id != media_session_id);
                    if refs.is_empty() {
                        sessions.remove(&(tenant_id, node_id));
                    }
                }
                Ok(MediaNodeCommandResult::Completed)
            }
            CommandPayload::ControlPlayback { .. } => Ok(MediaNodeCommandResult::Completed),
            CommandPayload::Ptz { .. }
            | CommandPayload::Query { .. }
            | CommandPayload::Preset { .. }
            | CommandPayload::DeviceControl { .. } => Err(DomainError::invalid_argument(
                "device command not dispatched through media node port",
            )),
        }
    }

    async fn list_nodes(
        &self,
        tenant_id: TenantId,
        _clock: &dyn Clock,
    ) -> crate::Result<Vec<crate::MediaNode>> {
        let node_sessions = lock_mutex(&self.node_sessions);
        Ok(node_sessions
            .iter()
            .filter(|((t, _), _)| *t == tenant_id)
            .map(|((_, node_id), refs)| crate::MediaNode {
                node_id: *node_id,
                session_count: refs.len() as u64,
                ..crate::MediaNode::default()
            })
            .collect())
    }

    async fn get_node(
        &self,
        node_id: NodeId,
        _clock: &dyn Clock,
    ) -> crate::Result<Option<crate::MediaNode>> {
        let scripted_nodes = lock_mutex(&self.scripted_nodes);
        Ok(scripted_nodes
            .values()
            .find(|node| node.node_id == node_id)
            .cloned())
    }

    async fn list_sessions(
        &self,
        tenant_id: TenantId,
        media_node_id: NodeId,
        page: PageRequest,
        _clock: &dyn Clock,
    ) -> crate::Result<Page<MediaNodeSessionRef>> {
        let sessions = lock_mutex(&self.node_sessions);
        let all_items = sessions
            .get(&(tenant_id, media_node_id))
            .cloned()
            .unwrap_or_default();
        let page_size = page.page_size as usize;
        let start = match &page.cursor {
            None => 0,
            Some(value) => value
                .parse::<usize>()
                .map_err(|_| DomainError::invalid_argument("invalid page cursor"))?,
        };
        let start = start.min(all_items.len());
        let end = (start + page_size).min(all_items.len());
        let items = all_items[start..end].to_vec();
        let next_cursor = if end < all_items.len() {
            Some(end.to_string())
        } else {
            None
        };
        Ok(Page {
            items,
            next_cursor,
            total: Some(all_items.len() as u64),
        })
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

/// In-memory [`ProtocolSessionRepository`] for tests.
///
/// Mirrors the SQL adapters' optimistic-concurrency and tenant-scoping
/// semantics: reads filter by [`TenantId`], [`save`](Self::save) applies the
/// `revision == stored + 1` guard, [`delete`](ProtocolSessionRepository::delete)
/// checks the expected revision, and [`list_expired`] pages in
/// `(updated_at, id)` order so the reaper sees a stable sweep.
#[derive(Clone, Debug, Default)]
pub struct InMemoryProtocolSessionRepository {
    sessions: Arc<Mutex<BTreeMap<uuid::Uuid, ProtocolSession>>>,
}

impl InMemoryProtocolSessionRepository {
    /// Creates an empty repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of stored sessions.
    pub fn len(&self) -> usize {
        lock_mutex(&self.sessions).len()
    }

    /// Returns `true` when no sessions are stored.
    pub fn is_empty(&self) -> bool {
        lock_mutex(&self.sessions).is_empty()
    }
}

#[async_trait::async_trait]
impl ProtocolSessionRepository for InMemoryProtocolSessionRepository {
    async fn get(
        &self,
        tenant_id: TenantId,
        protocol_session_id: ProtocolSessionId,
    ) -> crate::Result<Option<ProtocolSession>> {
        let sessions = lock_mutex(&self.sessions);
        Ok(sessions
            .get(&protocol_session_id.as_uuid())
            .filter(|s| s.tenant_id() == tenant_id)
            .cloned())
    }

    async fn get_by_device(
        &self,
        tenant_id: TenantId,
        protocol: Protocol,
        device_id: DeviceId,
    ) -> crate::Result<Option<ProtocolSession>> {
        let sessions = lock_mutex(&self.sessions);
        Ok(sessions
            .values()
            .find(|s| {
                s.tenant_id() == tenant_id && s.protocol() == protocol && s.device_id() == device_id
            })
            .cloned())
    }

    async fn get_by_identity(
        &self,
        tenant_id: TenantId,
        protocol: Protocol,
        protocol_identity: ProtocolIdentity,
    ) -> crate::Result<Option<ProtocolSession>> {
        let sessions = lock_mutex(&self.sessions);
        Ok(sessions
            .values()
            .find(|s| {
                s.tenant_id() == tenant_id
                    && s.protocol() == protocol
                    && s.protocol_identity() == &protocol_identity
            })
            .cloned())
    }

    async fn save(&mut self, session: &ProtocolSession) -> crate::Result<()> {
        let mut sessions = lock_mutex(&self.sessions);
        save_with_revision(
            &mut sessions,
            session.protocol_session_id().as_uuid(),
            session.clone(),
            ProtocolSession::revision,
        )
    }

    async fn delete(
        &mut self,
        tenant_id: TenantId,
        protocol_session_id: ProtocolSessionId,
        expected_revision: Revision,
    ) -> crate::Result<()> {
        let mut sessions = lock_mutex(&self.sessions);
        match sessions.get(&protocol_session_id.as_uuid()) {
            Some(existing)
                if existing.tenant_id() == tenant_id
                    && existing.revision() == expected_revision =>
            {
                sessions.remove(&protocol_session_id.as_uuid());
                Ok(())
            }
            Some(existing) if existing.tenant_id() == tenant_id => {
                Err(DomainError::ConcurrentModification {
                    expected: expected_revision.0,
                    found: existing.revision().0,
                })
            }
            _ => Err(DomainError::not_found(
                "protocol_session",
                protocol_session_id.as_uuid().to_string(),
            )),
        }
    }

    async fn list_expired(
        &self,
        now: UtcTimestamp,
        page: PageRequest,
    ) -> crate::Result<Page<ProtocolSession>> {
        let after = match &page.cursor {
            None => None,
            Some(value) => {
                let cursor = ListCursor::decode(value)
                    .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
                let (ts, id) = cursor
                    .parse()
                    .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
                Some((ts, id))
            }
        };

        let mut items: Vec<ProtocolSession> = {
            let sessions = lock_mutex(&self.sessions);
            sessions
                .values()
                .filter(|s| s.is_expired(now))
                .cloned()
                .collect()
        };
        items.sort_by(|a, b| {
            a.updated_at().cmp(&b.updated_at()).then_with(|| {
                a.protocol_session_id()
                    .as_uuid()
                    .cmp(&b.protocol_session_id().as_uuid())
            })
        });
        if let Some((ts, id)) = after {
            items.retain(|s| (s.updated_at(), s.protocol_session_id().as_uuid()) > (ts, id));
        }

        let page_size = page.page_size_as_usize();
        let has_more = items.len() > page_size;
        let next_cursor = if has_more {
            let last = items
                .get(page_size - 1)
                .ok_or_else(|| DomainError::internal("empty page"))?;
            Some(
                ListCursor::new(last.updated_at(), last.protocol_session_id().as_uuid())
                    .and_then(|c| c.encode())
                    .map_err(|e| DomainError::internal(format!("failed to encode cursor: {e}")))?,
            )
        } else {
            None
        };

        let mut result = Page::new(items.into_iter().take(page_size).collect());
        if let Some(cursor) = next_cursor {
            result = result.with_next_cursor(cursor);
        }
        Ok(result)
    }
}

/// In-memory [`PlatformLinkRepository`] for tests.
///
/// Mirrors the SQL adapters' optimistic-concurrency and tenant-scoping
/// semantics for GB28181 cascade platform links.
#[derive(Clone, Debug, Default)]
pub struct InMemoryPlatformLinkRepository {
    links: Arc<Mutex<BTreeMap<uuid::Uuid, GbPlatformLink>>>,
}

impl InMemoryPlatformLinkRepository {
    /// Creates an empty repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of stored links.
    pub fn len(&self) -> usize {
        lock_mutex(&self.links).len()
    }

    /// Returns `true` when no links are stored.
    pub fn is_empty(&self) -> bool {
        lock_mutex(&self.links).is_empty()
    }
}

#[async_trait::async_trait]
impl PlatformLinkRepository for InMemoryPlatformLinkRepository {
    async fn get(
        &self,
        tenant_id: TenantId,
        platform_link_id: PlatformLinkId,
    ) -> crate::Result<Option<GbPlatformLink>> {
        let links = lock_mutex(&self.links);
        Ok(links
            .get(&platform_link_id.as_uuid())
            .filter(|l| l.tenant_id() == tenant_id)
            .cloned())
    }

    async fn get_by_remote_identity(
        &self,
        tenant_id: TenantId,
        direction: PlatformDirection,
        remote_identity: ProtocolIdentity,
    ) -> crate::Result<Option<GbPlatformLink>> {
        let links = lock_mutex(&self.links);
        Ok(links
            .values()
            .find(|l| {
                l.tenant_id() == tenant_id
                    && l.direction() == direction
                    && l.identity().remote == remote_identity
            })
            .cloned())
    }

    async fn save(&mut self, link: &GbPlatformLink) -> crate::Result<()> {
        let mut links = lock_mutex(&self.links);
        save_with_revision(
            &mut links,
            link.platform_link_id().as_uuid(),
            link.clone(),
            GbPlatformLink::revision,
        )
    }

    async fn delete(
        &mut self,
        tenant_id: TenantId,
        platform_link_id: PlatformLinkId,
        expected_revision: Revision,
    ) -> crate::Result<()> {
        let mut links = lock_mutex(&self.links);
        match links.get(&platform_link_id.as_uuid()) {
            Some(existing)
                if existing.tenant_id() == tenant_id
                    && existing.revision() == expected_revision =>
            {
                links.remove(&platform_link_id.as_uuid());
                Ok(())
            }
            Some(existing) if existing.tenant_id() == tenant_id => {
                Err(DomainError::ConcurrentModification {
                    expected: expected_revision.0,
                    found: existing.revision().0,
                })
            }
            _ => Err(DomainError::not_found(
                "platform_link",
                platform_link_id.as_uuid().to_string(),
            )),
        }
    }

    async fn list(
        &self,
        tenant_id: TenantId,
        direction: Option<PlatformDirection>,
        page: PageRequest,
    ) -> crate::Result<Page<GbPlatformLink>> {
        let after = match &page.cursor {
            None => None,
            Some(value) => {
                let cursor = ListCursor::decode(value)
                    .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
                let (ts, id) = cursor
                    .parse()
                    .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
                Some((ts, id))
            }
        };

        let mut items: Vec<GbPlatformLink> = {
            let links = lock_mutex(&self.links);
            links
                .values()
                .filter(|l| {
                    l.tenant_id() == tenant_id
                        && direction.map(|d| l.direction() == d).unwrap_or(true)
                })
                .cloned()
                .collect()
        };
        items.sort_by(|a, b| {
            a.updated_at().cmp(&b.updated_at()).then_with(|| {
                a.platform_link_id()
                    .as_uuid()
                    .cmp(&b.platform_link_id().as_uuid())
            })
        });
        if let Some((ts, id)) = after {
            items.retain(|l| (l.updated_at(), l.platform_link_id().as_uuid()) > (ts, id));
        }

        let page_size = page.page_size_as_usize();
        let has_more = items.len() > page_size;
        let next_cursor = if has_more {
            let last = items
                .get(page_size - 1)
                .ok_or_else(|| DomainError::internal("empty page"))?;
            Some(
                ListCursor::new(last.updated_at(), last.platform_link_id().as_uuid())
                    .and_then(|c| c.encode())
                    .map_err(|e| DomainError::internal(format!("failed to encode cursor: {e}")))?,
            )
        } else {
            None
        };

        let mut result = Page::new(items.into_iter().take(page_size).collect());
        if let Some(cursor) = next_cursor {
            result = result.with_next_cursor(cursor);
        }
        Ok(result)
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
        source_ip: None,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::MediaPort;

    #[test]
    fn list_sessions_rejects_invalid_cursor() {
        let id_generator = InMemoryIdGenerator::new();
        let port = InMemoryMediaPort::new(Arc::new(id_generator));
        let tenant_id = TenantId::from_uuid(uuid::Uuid::nil());
        let node_id = NodeId::from_uuid(uuid::Uuid::nil());

        let result = futures::executor::block_on(port.list_sessions(
            tenant_id,
            node_id,
            PageRequest::new(10).unwrap().with_cursor("not-a-number"),
            &InMemoryClock::new(),
        ));
        assert!(
            matches!(result, Err(DomainError::InvalidArgument { .. })),
            "invalid cursor must be rejected, got {:?}",
            result
        );
    }
}

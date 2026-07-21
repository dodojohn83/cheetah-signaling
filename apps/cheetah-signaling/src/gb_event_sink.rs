//! GB28181 application event sink.
//!
//! Routes incoming GB28181 driver events into the application service layer
//! instead of only logging them. The sink is non-blocking: it drops events
//! when the bounded channel is full and processes them asynchronously in a
//! background worker.

use cheetah_domain::{Connectivity, Device, Protocol};
use cheetah_gb28181_driver_tokio::sink::EventSink;
use cheetah_gb28181_module::DeviceId as GbDeviceId;
use cheetah_gb28181_module::Gb28181Event;
use cheetah_gb28181_module::xml::CatalogItem as GbCatalogItem;
use cheetah_http_api::metrics::RequestMetrics;
use cheetah_http_api::state::ApiState;
use cheetah_signal_application::{
    ChannelDescriptor, MarkDeviceOfflineRequest, MarkDeviceOnlineRequest, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest, UpdateDeviceCapabilitiesRequest,
};
use cheetah_signal_types::{
    ChannelId, CorrelationId, DeviceId, MessageId, NodeId, Principal, PrincipalKind,
    ProtocolIdentity, RequestContext, SignalError, SignalErrorKind, TenantId,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

/// Non-blocking event sink that forwards `Gb28181Event`s to a background
/// worker for processing through the application service layer.
#[derive(Clone, Debug)]
pub struct GbApplicationEventSink {
    tx: mpsc::Sender<Gb28181Event>,
    metrics: Arc<RequestMetrics>,
}

impl EventSink<Gb28181Event> for GbApplicationEventSink {
    fn emit(&self, event: Gb28181Event) {
        if let Err(e) = self.tx.try_send(event) {
            self.metrics.record_gb28181_event_dropped();
            warn!(error = %e, "gb28181 event sink full; dropping event");
        }
    }
}

/// Spawns a background worker that consumes GB28181 events and applies them
/// through `DeviceService` using bounded in-memory queueing. Returns the sink
/// to be given to the UDP driver and a handle to the spawned worker.
pub fn spawn(
    state: ApiState,
    node_id: NodeId,
    tenant_id: Option<TenantId>,
    queue_depth: usize,
    catalog_max_entries: usize,
    catalog_max_items: usize,
    cancel: tokio_util::sync::CancellationToken,
) -> (
    Arc<dyn EventSink<Gb28181Event>>,
    tokio::task::JoinHandle<()>,
) {
    let queue_depth = queue_depth.max(1);
    let (tx, mut rx) = mpsc::channel(queue_depth);
    let metrics = state.metrics.clone();
    let sink = Arc::new(GbApplicationEventSink { tx, metrics }) as Arc<dyn EventSink<Gb28181Event>>;
    let mut catalog_buffer = CatalogBuffer::new(catalog_max_entries, catalog_max_items);
    let mut cleanup = tokio::time::interval(CATALOG_CLEANUP_INTERVAL);
    cleanup.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = cleanup.tick() => {
                    catalog_buffer.evict();
                    continue;
                }
                maybe_event = rx.recv() => {
                    match maybe_event {
                        Some(event) => process_event(&state, node_id, tenant_id, event, &mut catalog_buffer).await,
                        None => break,
                    }
                }
            }
        }
        info!("gb28181 application event sink stopped");
    });
    (sink, handle)
}

async fn process_event(
    state: &ApiState,
    node_id: NodeId,
    tenant_id: Option<TenantId>,
    event: Gb28181Event,
    catalog_buffer: &mut CatalogBuffer,
) {
    let tenant_id = match tenant_id {
        Some(id) => id,
        None => {
            warn!("dropping gb28181 event; no default_tenant_id configured");
            return;
        }
    };

    let context = build_context(state, node_id, tenant_id, &event);

    let result = match event {
        Gb28181Event::DeviceRegistered { device_id, .. } => {
            ensure_online(state, &context, tenant_id, &device_id, true).await
        }
        Gb28181Event::DeviceUnregistered { device_id, .. } => {
            mark_offline(state, &context, tenant_id, &device_id).await
        }
        Gb28181Event::DevicePresenceChanged {
            device_id,
            presence,
            ..
        } => match presence {
            cheetah_gb28181_module::DevicePresence::Online => {
                ensure_online(state, &context, tenant_id, &device_id, true).await
            }
            cheetah_gb28181_module::DevicePresence::Offline => {
                mark_offline(state, &context, tenant_id, &device_id).await
            }
        },
        Gb28181Event::Keepalive { device_id, .. } => {
            ensure_online(state, &context, tenant_id, &device_id, false).await
        }
        Gb28181Event::CatalogReceived {
            device_id,
            sn,
            sum_num,
            num,
            items,
            ..
        } => match catalog_buffer.accumulate(tenant_id, &device_id, &sn, sum_num, num, items) {
            Some(merged) => replace_catalog(state, &context, tenant_id, &device_id, &merged).await,
            None => Ok(()),
        },
        Gb28181Event::DeviceInfoReceived {
            device_id,
            result,
            manufacturer,
            model,
            firmware,
            ..
        } => {
            let mut metadata = BTreeMap::new();
            if let Some(v) = result {
                metadata.insert("info_result".to_string(), v.clone());
            }
            if let Some(v) = manufacturer {
                metadata.insert("manufacturer".to_string(), v.clone());
            }
            if let Some(v) = model {
                metadata.insert("model".to_string(), v.clone());
            }
            if let Some(v) = firmware {
                metadata.insert("firmware".to_string(), v.clone());
            }
            update_device_info(state, &context, tenant_id, &device_id, metadata).await
        }
        Gb28181Event::DeviceStatusReceived {
            device_id,
            result,
            online,
            status,
            reason,
            invalid_equip,
            ..
        } => {
            let mut metadata = BTreeMap::new();
            if let Some(v) = result {
                metadata.insert("status_result".to_string(), v.clone());
            }
            if let Some(v) = online {
                metadata.insert("online".to_string(), v.clone());
            }
            if let Some(v) = status {
                metadata.insert("status".to_string(), v.clone());
            }
            if let Some(v) = reason {
                metadata.insert("reason".to_string(), v.clone());
            }
            if let Some(v) = invalid_equip {
                metadata.insert("invalid_equip".to_string(), v.clone());
            }
            update_device_info(state, &context, tenant_id, &device_id, metadata).await
        }
        Gb28181Event::AlarmReceived { device_id, .. } => {
            info!(%device_id, "gb28181 alarm received; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::MobilePositionReceived { device_id, .. } => {
            info!(%device_id, "gb28181 mobile position received; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::DeviceControlResponseReceived {
            device_id,
            sn,
            result,
            ..
        } => {
            info!(%device_id, %sn, ?result, "gb28181 control response received; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::MediaSessionStarted {
            media_session_id,
            device_id,
            channel_id,
            ..
        } => {
            info!(%media_session_id, %device_id, %channel_id, "gb28181 media session started; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::MediaSessionStopped {
            media_session_id,
            device_id,
            channel_id,
            ..
        } => {
            info!(%media_session_id, %device_id, %channel_id, "gb28181 media session stopped; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::MediaSessionFailed {
            media_session_id,
            device_id,
            channel_id,
            reason,
            ..
        } => {
            info!(%media_session_id, %device_id, %channel_id, %reason, "gb28181 media session failed; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::RecordInfoReceived { device_id, .. } => {
            info!(%device_id, "gb28181 record info received; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::CascadePlatformConnected { platform_id, .. } => {
            info!(%platform_id, "gb28181 cascade platform connected; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::CascadePlatformDisconnected { platform_id, .. } => {
            info!(%platform_id, "gb28181 cascade platform disconnected; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::CascadePlayRequested { bridge_id, .. } => {
            info!(%bridge_id, "gb28181 cascade play requested; no application handler wired yet");
            Ok(())
        }
        Gb28181Event::CascadePlayStopped { bridge_id, .. } => {
            info!(%bridge_id, "gb28181 cascade play stopped; no application handler wired yet");
            Ok(())
        }
    };

    if let Err(e) = result {
        warn!(error = %e, "failed to process gb28181 event");
    }
}

fn build_context(
    _state: &ApiState,
    node_id: NodeId,
    tenant_id: TenantId,
    event: &Gb28181Event,
) -> RequestContext {
    let source_ip = event_source(event).map(|s| s.ip().to_string());
    RequestContext {
        tenant_id,
        principal: Principal {
            id: "gb28181".to_string(),
            kind: PrincipalKind::Service,
            scopes: vec!["device:write".to_string()],
        },
        message_id: MessageId::from_uuid(Uuid::now_v7()),
        correlation_id: CorrelationId::from_uuid(Uuid::now_v7()),
        traceparent: None,
        tracestate: None,
        deadline: None,
        node_id: Some(node_id),
        source_ip,
    }
}

fn event_source(event: &Gb28181Event) -> Option<&std::net::SocketAddr> {
    match event {
        Gb28181Event::DeviceRegistered { source, .. } => Some(source),
        Gb28181Event::DeviceUnregistered { source, .. } => Some(source),
        Gb28181Event::DevicePresenceChanged { source, .. } => Some(source),
        Gb28181Event::Keepalive { source, .. } => Some(source),
        Gb28181Event::CatalogReceived { source, .. } => Some(source),
        Gb28181Event::DeviceInfoReceived { source, .. } => Some(source),
        Gb28181Event::DeviceStatusReceived { source, .. } => Some(source),
        Gb28181Event::AlarmReceived { source, .. } => Some(source),
        Gb28181Event::MobilePositionReceived { source, .. } => Some(source),
        Gb28181Event::DeviceControlResponseReceived { source, .. } => Some(source),
        Gb28181Event::MediaSessionStarted { source, .. } => Some(source),
        Gb28181Event::MediaSessionStopped { source, .. } => source.as_ref(),
        Gb28181Event::MediaSessionFailed { source, .. } => source.as_ref(),
        Gb28181Event::RecordInfoReceived { source, .. } => Some(source),
        _ => None,
    }
}

/// Creates a stable key for a catalog fragment from the sorted set of channel
/// external ids it contains. Retransmissions of the same fragment will share
/// the same key and therefore contribute their declared `Num` only once.
fn fragment_key(batch: &HashMap<String, GbCatalogItem>) -> String {
    if batch.is_empty() {
        return "empty".to_string();
    }
    let mut ids: Vec<_> = batch.keys().map(String::as_str).collect();
    ids.sort();
    ids.join(",")
}

const CATALOG_FRAGMENT_TTL: Duration = Duration::from_secs(60);
const CATALOG_CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

/// In-memory accumulator for paginated GB28181 catalog fragments.
///
/// Devices may split a catalog response across multiple SIP MESSAGE bodies.
/// Fragments are keyed by (tenant, device, sequence number) and merged into a
/// single `replace_channel_catalog` call once `sum_num` distinct channel ids
/// have been seen. Items are de-duplicated by their channel `device_id`.
///
/// To avoid stalling when a camera drops malformed items, completion also falls
/// back to the sum of declared `Num` values for each *unique* fragment content
/// (retransmissions are ignored). If the unique fragment count equals `sum_num`
/// but fewer distinct channels were collected, the partial catalog is emitted as
/// a best-effort replacement and a warning is logged. Overlapping fragments that
/// would push the unique declared count above `sum_num` are not used to trigger
/// completion. Partial transfers expire after `CATALOG_FRAGMENT_TTL` and are
/// evicted by the background worker cleanup tick.
struct CatalogBuffer {
    entries: HashMap<CatalogKey, PartialCatalog>,
    max_entries: usize,
    max_items_per_entry: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct CatalogKey {
    tenant_id: TenantId,
    device_id: String,
    sn: String,
}

struct PartialCatalog {
    /// Accumulated catalog items keyed by channel external id (`device_id`).
    items: HashMap<String, GbCatalogItem>,
    /// Declared total number of items across all fragments (`SumNum`).
    expected: u32,
    /// Declared `Num` values keyed by a digest of the fragment's channel ids.
    ///
    /// Retransmissions of the same fragment share the same key and do not
    /// contribute multiple times. For each unique fragment the largest `Num`
    /// value observed is kept.
    fragments: HashMap<String, u32>,
    last_seen: Instant,
}

impl PartialCatalog {
    /// Sum of declared `Num` values for unique fragments received so far.
    fn received_num(&self) -> u32 {
        self.fragments
            .values()
            .copied()
            .fold(0u32, |acc, v| acc.saturating_add(v))
    }

    fn is_complete(&self) -> bool {
        let distinct = self.items.len();
        if distinct >= self.expected as usize {
            if distinct > self.expected as usize {
                warn!(
                    expected = self.expected,
                    distinct, "gb28181 catalog has more distinct channels than declared"
                );
            }
            return true;
        }

        let received = self.received_num();
        if received == self.expected {
            warn!(
                expected = self.expected,
                distinct,
                received,
                "gb28181 catalog unique fragment count reached sum_num with fewer distinct channels; some items may have been malformed or dropped"
            );
            return true;
        }

        if received > self.expected {
            warn!(
                expected = self.expected,
                distinct,
                received,
                "gb28181 catalog fragments overlap or repeat declared counts; waiting for distinct channel ids"
            );
        }

        false
    }
}

impl CatalogBuffer {
    fn new(max_entries: usize, max_items_per_entry: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            max_items_per_entry,
        }
    }

    fn accumulate(
        &mut self,
        tenant_id: TenantId,
        device_id: &GbDeviceId,
        sn: &str,
        expected: u32,
        num: u32,
        items: Vec<GbCatalogItem>,
    ) -> Option<Vec<GbCatalogItem>> {
        // De-duplicate within the incoming fragment before any size checks.
        let mut batch = HashMap::with_capacity(items.len());
        for item in items {
            batch.insert(item.device_id.clone(), item);
        }

        if expected == 0 {
            return Some(batch.into_values().collect());
        }

        let expected_usize = expected as usize;
        if expected_usize > self.max_items_per_entry {
            warn!(
                %device_id,
                sn,
                expected,
                max_items_per_entry = self.max_items_per_entry,
                "gb28181 catalog fragment declares more items than allowed; dropping"
            );
            return None;
        }

        let key = CatalogKey {
            tenant_id,
            device_id: device_id.as_ref().to_string(),
            sn: sn.to_string(),
        };

        if !self.entries.contains_key(&key) && self.entries.len() >= self.max_entries {
            warn!(
                sn,
                max_entries = self.max_entries,
                "gb28181 catalog fragment buffer full; dropping new fragment"
            );
            return None;
        }

        if let Some(partial) = self.entries.get_mut(&key) {
            let new_distinct = batch
                .keys()
                .filter(|k| !partial.items.contains_key(*k))
                .count();
            let total = partial.items.len().saturating_add(new_distinct);
            if total > self.max_items_per_entry {
                warn!(
                    %device_id,
                    sn,
                    accumulated = total,
                    max_items_per_entry = self.max_items_per_entry,
                    "gb28181 catalog fragment exceeded per-entry item limit; dropping partial"
                );
                self.entries.remove(&key);
                return None;
            }
            partial
                .fragments
                .entry(fragment_key(&batch))
                .and_modify(|v| *v = num.max(*v))
                .or_insert(num);
            partial.items.extend(batch);
            partial.last_seen = Instant::now();
            if partial.is_complete() {
                return self
                    .entries
                    .remove(&key)
                    .map(|complete| complete.items.into_values().collect());
            }
            return None;
        }

        if batch.len() > self.max_items_per_entry {
            warn!(
                %device_id,
                sn,
                accumulated = batch.len(),
                max_items_per_entry = self.max_items_per_entry,
                "gb28181 catalog fragment exceeded per-entry item limit; dropping"
            );
            return None;
        }

        let mut fragments = HashMap::new();
        if num > 0 || !batch.is_empty() {
            fragments.insert(fragment_key(&batch), num);
        }
        let partial = PartialCatalog {
            items: batch,
            expected,
            fragments,
            last_seen: Instant::now(),
        };
        if partial.is_complete() {
            return Some(partial.items.into_values().collect());
        }
        self.entries.insert(key, partial);
        None
    }

    fn evict(&mut self) {
        let now = Instant::now();
        let before = self.entries.len();
        self.entries
            .retain(|_, partial| now.duration_since(partial.last_seen) <= CATALOG_FRAGMENT_TTL);
        let dropped = before.saturating_sub(self.entries.len());
        if dropped > 0 {
            warn!(
                dropped,
                "gb28181 catalog fragment buffer evicted stale entries"
            );
        }
    }
}

fn storage_error(e: cheetah_storage_api::StorageError) -> SignalError {
    SignalError::new(
        SignalErrorKind::Internal,
        format!("failed to begin storage transaction: {e}"),
    )
}

async fn resolve_device(
    state: &ApiState,
    tenant_id: TenantId,
    external_id: &str,
) -> Option<Device> {
    let mut uow = match state.storage.begin().await {
        Ok(u) => u,
        Err(e) => {
            warn!(error = %e, "failed to begin unit of work");
            return None;
        }
    };
    let identity = match ProtocolIdentity::new(external_id) {
        Ok(i) => i,
        Err(e) => {
            warn!(error = %e, external_id, "invalid gb28181 external id");
            return None;
        }
    };
    match uow
        .device_repository()
        .get_by_external_id(tenant_id, Protocol::Gb28181, identity)
        .await
    {
        Ok(device) => device,
        Err(e) => {
            warn!(error = %e, external_id, "failed to resolve gb28181 device");
            None
        }
    }
}

async fn resolve_device_id(
    state: &ApiState,
    tenant_id: TenantId,
    external_id: &str,
) -> Option<DeviceId> {
    resolve_device(state, tenant_id, external_id)
        .await
        .map(|d| d.device_id())
}

async fn ensure_online(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: &GbDeviceId,
    force: bool,
) -> Result<(), SignalError> {
    let external_id = device_id.as_ref();
    if let Some(device) = resolve_device(state, tenant_id, external_id).await {
        if force || !matches!(device.connectivity(), Connectivity::Online) {
            let mut uow = state.storage.begin().await.map_err(storage_error)?;
            let _ = state
                .device_service
                .mark_device_online(
                    context,
                    &mut *uow,
                    device.device_id(),
                    MarkDeviceOnlineRequest {
                        reason: Some("gb28181 online".to_string()),
                    },
                )
                .await?;
        }
        return Ok(());
    }

    if !force {
        return Ok(());
    }

    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let result = state
        .device_service
        .register_or_update_device(
            context,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: external_id.to_string(),
                authority: Some("gb28181".to_string()),
                name: external_id.to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await?;

    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let _ = state
        .device_service
        .mark_device_online(
            context,
            &mut *uow,
            result.device.device_id,
            MarkDeviceOnlineRequest {
                reason: Some("gb28181 registered".to_string()),
            },
        )
        .await?;
    Ok(())
}

async fn mark_offline(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: &GbDeviceId,
) -> Result<(), SignalError> {
    let external_id = device_id.as_ref();
    if let Some(internal_id) = resolve_device_id(state, tenant_id, external_id).await {
        let mut uow = state.storage.begin().await.map_err(storage_error)?;
        let _ = state
            .device_service
            .mark_device_offline(
                context,
                &mut *uow,
                internal_id,
                MarkDeviceOfflineRequest {
                    reason: "gb28181 offline".to_string(),
                },
            )
            .await?;
    }
    Ok(())
}

async fn update_device_info(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: &GbDeviceId,
    metadata: BTreeMap<String, String>,
) -> Result<(), SignalError> {
    let external_id = device_id.as_ref();
    let device = match resolve_device(state, tenant_id, external_id).await {
        Some(d) => d,
        None => return Ok(()),
    };

    let mut merged = device.metadata().clone();
    for (k, v) in metadata {
        merged.insert(k, v);
    }

    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let _ = state
        .device_service
        .update_device_capabilities(
            context,
            &mut *uow,
            device.device_id(),
            device.revision(),
            UpdateDeviceCapabilitiesRequest {
                capabilities: None,
                metadata: Some(merged),
            },
        )
        .await?;
    Ok(())
}

async fn replace_catalog(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: &GbDeviceId,
    items: &[GbCatalogItem],
) -> Result<(), SignalError> {
    let external_id = device_id.as_ref();
    let internal_id = match resolve_device_id(state, tenant_id, external_id).await {
        Some(id) => id,
        None => return Ok(()),
    };

    let mut channels = Vec::with_capacity(items.len());
    for item in items {
        let channel_id = catalog_channel_id(tenant_id, external_id, &item.device_id);
        let mut metadata = BTreeMap::new();
        if let Some(v) = &item.manufacturer {
            metadata.insert("manufacturer".to_string(), v.clone());
        }
        if let Some(v) = &item.model {
            metadata.insert("model".to_string(), v.clone());
        }
        if let Some(v) = &item.owner {
            metadata.insert("owner".to_string(), v.clone());
        }
        if let Some(v) = &item.civil_code {
            metadata.insert("civil_code".to_string(), v.clone());
        }
        if let Some(v) = &item.block {
            metadata.insert("block".to_string(), v.clone());
        }
        if let Some(v) = &item.address {
            metadata.insert("address".to_string(), v.clone());
        }
        if let Some(v) = &item.parent_id {
            metadata.insert("parent_id".to_string(), v.clone());
        }
        if let Some(v) = &item.ip_address {
            metadata.insert("ip_address".to_string(), v.clone());
        }
        if let Some(v) = &item.port {
            metadata.insert("port".to_string(), v.clone());
        }
        if let Some(v) = &item.status {
            metadata.insert("status".to_string(), v.clone());
        }
        if let Some(v) = &item.longitude {
            metadata.insert("longitude".to_string(), v.clone());
        }
        if let Some(v) = &item.latitude {
            metadata.insert("latitude".to_string(), v.clone());
        }

        channels.push(ChannelDescriptor {
            id: Some(channel_id.to_string()),
            name: item.name.clone().unwrap_or_else(|| item.device_id.clone()),
            kind: "video".to_string(),
            enabled: true,
            status: None,
            stream_profiles: vec![],
            ptz_capabilities: None,
            metadata: Some(metadata),
        });
    }

    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let _ = state
        .device_service
        .replace_channel_catalog(
            context,
            &mut *uow,
            internal_id,
            ReplaceChannelCatalogRequest { channels },
        )
        .await?;
    Ok(())
}

fn catalog_channel_id(
    tenant_id: TenantId,
    device_external_id: &str,
    channel_external_id: &str,
) -> ChannelId {
    let namespace = Uuid::NAMESPACE_OID;
    let name = format!(
        "gb28181/{}/{}/{}",
        tenant_id, device_external_id, channel_external_id
    );
    ChannelId::from_uuid(Uuid::new_v5(&namespace, name.as_bytes()))
}

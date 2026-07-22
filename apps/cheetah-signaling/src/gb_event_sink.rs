//! GB28181 application event sink.
//!
//! Routes incoming GB28181 driver events into the application service layer
//! instead of only logging them. The sink is non-blocking: it drops events
//! when the bounded channel is full and processes them asynchronously in a
//! background worker.

use cheetah_domain::{
    Connectivity, Device, DomainEvent, MediaSession, MediaSessionError, MediaSessionState,
    Protocol, UnitOfWork,
};
use cheetah_gb28181_driver_tokio::sink::EventSink;
use cheetah_gb28181_module::DeviceId as GbDeviceId;
use cheetah_gb28181_module::Gb28181Event;
use cheetah_gb28181_module::bootstrap;
use cheetah_gb28181_module::xml::{CatalogItem as GbCatalogItem, RecordItem as GbRecordItem};
use cheetah_http_api::metrics::RequestMetrics;
use cheetah_http_api::state::ApiState;
use cheetah_signal_application::{
    ChannelDescriptor, MarkDeviceOfflineRequest, MarkDeviceOnlineRequest, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest, SubmitOperationRequest, UpdateDeviceCapabilitiesRequest,
};
use cheetah_signal_types::{
    CorrelationId, Deadline, DeviceId, DurationMs, Event, GbCommandMethod, GbCommandOutcome,
    GbMetricsRecorder, MediaSessionId, MessageId, NodeId, OwnerEpoch, Principal, PrincipalKind,
    ProtocolIdentity, RequestContext, ResourceId, ResourceKind, ResourceRef, SignalError,
    SignalErrorKind, TenantId,
};
use std::collections::BTreeMap;
use std::sync::Arc;
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
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    state: ApiState,
    node_id: NodeId,
    tenant_id: Option<TenantId>,
    queue_depth: usize,
    catalog_max_entries: usize,
    catalog_max_items: usize,
    record_max_entries: usize,
    record_max_items: usize,
    gb_metrics: Arc<dyn GbMetricsRecorder>,
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
    let mut record_buffer = RecordInfoBuffer::new(record_max_entries, record_max_items);
    let mut cleanup = tokio::time::interval(CATALOG_CLEANUP_INTERVAL);
    cleanup.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = cleanup.tick() => {
                    catalog_buffer.evict();
                    record_buffer.evict();
                    continue;
                }
                maybe_event = rx.recv() => {
                    match maybe_event {
                        Some(event) => {
                            if let Err(e) = process_event(&state, node_id, tenant_id, event, &mut catalog_buffer, &mut record_buffer, gb_metrics.as_ref()).await {
                                warn!(error = %e, "failed to process gb28181 event");
                            }
                        }
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
    record_buffer: &mut RecordInfoBuffer,
    gb_metrics: &dyn GbMetricsRecorder,
) -> Result<(), SignalError> {
    let tenant_id = match tenant_id {
        Some(id) => id,
        None => {
            warn!("dropping gb28181 event; no default_tenant_id configured");
            return Ok(());
        }
    };

    let context = build_context(state, node_id, tenant_id, &event);

    match event {
        Gb28181Event::DeviceRegistered {
            device_id,
            registration_sequence,
            ..
        } => match ensure_online(state, &context, tenant_id, &device_id, true).await {
            Ok(Some(internal_id)) => {
                submit_bootstrap_queries(
                    state,
                    &context,
                    tenant_id,
                    internal_id,
                    registration_sequence,
                )
                .await
            }
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        },
        Gb28181Event::DeviceUnregistered { device_id, .. } => {
            mark_offline(state, &context, tenant_id, &device_id).await
        }
        Gb28181Event::DevicePresenceChanged {
            device_id,
            presence,
            ..
        } => match presence {
            cheetah_gb28181_module::DevicePresence::Online => {
                ensure_online(state, &context, tenant_id, &device_id, true)
                    .await
                    .map(|_| ())
            }
            cheetah_gb28181_module::DevicePresence::Offline => {
                mark_offline(state, &context, tenant_id, &device_id).await
            }
        },
        Gb28181Event::Keepalive { device_id, .. } => {
            ensure_online(state, &context, tenant_id, &device_id, false)
                .await
                .map(|_| ())
        }
        Gb28181Event::CatalogReceived {
            device_id,
            sn,
            sum_num,
            num,
            items,
            ..
        } => {
            gb_metrics.record_catalog_fragment();
            match catalog_buffer.accumulate(tenant_id, &device_id, &sn, sum_num, num, items) {
                Some(merged) => {
                    replace_catalog(state, &context, tenant_id, &device_id, &merged).await
                }
                None => Ok(()),
            }
        }
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
        Gb28181Event::AlarmReceived {
            device_id,
            sn,
            priority,
            method,
            alarm_type,
            time,
            info,
            ..
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = priority {
                payload.insert("priority".to_string(), v);
            }
            if let Some(v) = method {
                payload.insert("method".to_string(), v);
            }
            if let Some(v) = alarm_type {
                payload.insert("alarm_type".to_string(), v);
            }
            if let Some(v) = time {
                payload.insert("time".to_string(), v);
            }
            if let Some(v) = info {
                payload.insert("info".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "Alarm",
                payload,
            )
            .await
        }
        Gb28181Event::MobilePositionReceived {
            device_id,
            sn,
            time,
            longitude,
            latitude,
            speed,
            direction,
            altitude,
            ..
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = time {
                payload.insert("time".to_string(), v);
            }
            if let Some(v) = longitude {
                payload.insert("longitude".to_string(), v);
            }
            if let Some(v) = latitude {
                payload.insert("latitude".to_string(), v);
            }
            if let Some(v) = speed {
                payload.insert("speed".to_string(), v);
            }
            if let Some(v) = direction {
                payload.insert("direction".to_string(), v);
            }
            if let Some(v) = altitude {
                payload.insert("altitude".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "MobilePosition",
                payload,
            )
            .await
        }
        Gb28181Event::DeviceControlResponseReceived {
            device_id,
            sn,
            result,
            ..
        } => {
            gb_metrics.record_command(GbCommandMethod::DeviceControl, control_outcome(&result));
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = result {
                payload.insert("result".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "DeviceControl",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionStarted {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
            // Raw negotiated SDP body is intentionally not persisted; only
            // bounded, desensitized control fields are recorded (AGENTS.md §8).
            remote_sdp: _,
            remote_ssrc,
            remote_port,
            remote_proto,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            payload.insert("remote_address".to_string(), source.to_string());
            if let Some(v) = remote_ssrc {
                payload.insert("remote_ssrc".to_string(), v);
            }
            payload.insert("remote_port".to_string(), remote_port.to_string());
            payload.insert("remote_proto".to_string(), remote_proto);
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Start,
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionStarted",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionStopped {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            if let Some(s) = source {
                payload.insert("remote_address".to_string(), s.to_string());
            }
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Stop,
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionStopped",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionFailed {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
            reason,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            if let Some(s) = source {
                payload.insert("remote_address".to_string(), s.to_string());
            }
            payload.insert("reason".to_string(), reason.clone());
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Fail(reason),
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionFailed",
                payload,
            )
            .await
        }
        Gb28181Event::RecordInfoReceived {
            device_id,
            sn,
            num,
            sum_num,
            items,
            ..
        } => {
            if let Some(records) =
                record_buffer.accumulate(tenant_id, &device_id, &sn, sum_num, num, items)
            {
                let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
                let mut payload = BTreeMap::new();
                payload.insert("sn".to_string(), sn);
                payload.insert("sum_num".to_string(), sum_num.to_string());
                payload.insert("num".to_string(), num.to_string());
                payload.insert("record_count".to_string(), records.len().to_string());
                if !records.is_empty()
                    && let Some(json) = serialize_record_items(&records)
                {
                    payload.insert("records".to_string(), json);
                }
                append_gb_event(
                    state,
                    &context,
                    tenant_id,
                    internal_id,
                    Some(device_id.as_ref()),
                    "RecordInfo",
                    payload,
                )
                .await?;
            }
            Ok(())
        }
        Gb28181Event::CascadePlatformConnected {
            domain_id,
            platform_id,
            upstream,
            expires,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("upstream".to_string(), upstream);
            payload.insert("expires".to_string(), expires.to_string());
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlatformConnected",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlatformDisconnected {
            domain_id,
            platform_id,
            reason,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("reason".to_string(), reason);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlatformDisconnected",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlayRequested {
            domain_id,
            platform_id,
            bridge_id,
            upstream_call_id,
            upstream_from,
            upstream_to,
            target_user,
            // Raw negotiated SDP body is intentionally not persisted; only
            // bounded, desensitized control fields are recorded (AGENTS.md §8).
            remote_sdp: _,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("bridge_id".to_string(), bridge_id.clone());
            payload.insert("upstream_call_id".to_string(), upstream_call_id);
            payload.insert("upstream_from".to_string(), upstream_from);
            payload.insert("upstream_to".to_string(), upstream_to);
            payload.insert("target_user".to_string(), target_user);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlayRequested",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlayStopped {
            domain_id,
            platform_id,
            bridge_id,
            reason,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("bridge_id".to_string(), bridge_id.clone());
            payload.insert("reason".to_string(), reason);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlayStopped",
                payload,
            )
            .await
        }
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

use crate::gb_catalog_buffer::{CATALOG_CLEANUP_INTERVAL, CatalogBuffer, RecordInfoBuffer};

/// Maps a GB28181 DeviceControl response result string to a bounded outcome.
fn control_outcome(result: &Option<String>) -> GbCommandOutcome {
    match result {
        Some(value) if value.eq_ignore_ascii_case("OK") => GbCommandOutcome::Succeeded,
        Some(_) => GbCommandOutcome::Failed,
        None => GbCommandOutcome::Unknown,
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
) -> Result<Option<DeviceId>, SignalError> {
    let external_id = device_id.as_ref();
    if let Some(device) = resolve_device(state, tenant_id, external_id).await {
        let internal_id = device.device_id();
        if force || !matches!(device.connectivity(), Connectivity::Online) {
            let mut uow = state.storage.begin().await.map_err(storage_error)?;
            let _ = state
                .device_service
                .mark_device_online(
                    context,
                    &mut *uow,
                    internal_id,
                    MarkDeviceOnlineRequest {
                        reason: Some("gb28181 online".to_string()),
                    },
                )
                .await?;
        }
        return Ok(Some(internal_id));
    }

    if !force {
        return Ok(None);
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

    let internal_id = result.device.device_id;
    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let _ = state
        .device_service
        .mark_device_online(
            context,
            &mut *uow,
            internal_id,
            MarkDeviceOnlineRequest {
                reason: Some("gb28181 registered".to_string()),
            },
        )
        .await?;
    Ok(Some(internal_id))
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

/// Submits Catalog / DeviceInfo / DeviceStatus bootstrap query Operations for a
/// freshly registered/online GB28181 device.
///
/// Each query uses a tenant-scoped, owner-epoch-qualified and
/// registration-sequence-qualified idempotency key so that the same
/// registration does not create duplicate operations while a new owner or a new
/// registration still spawns fresh queries.
async fn submit_bootstrap_queries(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: DeviceId,
    registration_sequence: u64,
) -> Result<(), SignalError> {
    let owner = state
        .owner_resolver
        .resolve(tenant_id, device_id)
        .await
        .map_err(SignalError::from)?;
    // A newly acquired owner always starts at epoch 1; defaulting to 0 would
    // cause any later owner-recovered operation to fail the epoch fence.
    let owner_epoch = owner.map(|o| o.owner_epoch).unwrap_or(OwnerEpoch(1));

    let deadline = Deadline::from_now(state.clock.now_wall(), DurationMs::from_seconds(30))
        .or_else(|| {
            warn!("bootstrap query deadline overflowed; proceeding without deadline");
            None
        });
    let target = ResourceRef {
        tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    };

    for kind in bootstrap::bootstrap_query_kinds() {
        let payload = bootstrap::bootstrap_query_payload(*kind);
        let idempotency_key = bootstrap::bootstrap_idempotency_key(
            tenant_id,
            device_id,
            owner_epoch,
            registration_sequence,
            *kind,
        );
        let mut uow = state.storage.begin().await.map_err(storage_error)?;
        let _ = state
            .operation_service
            .submit_operation(
                context,
                &mut *uow,
                SubmitOperationRequest {
                    device_id,
                    target: target.clone(),
                    payload,
                    idempotency_key,
                    deadline,
                    expected_owner_epoch: owner_epoch,
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
        let channel_id = cheetah_domain::channel::map_gb28181_channel_id(
            tenant_id,
            external_id,
            &item.device_id,
        );
        let mut metadata = BTreeMap::new();
        // Persist the GB28181 channel external ID so channel-scoped commands
        // (PTZ, preset, device control, record info) can address the SIP target.
        metadata.insert("external_id".to_string(), item.device_id.clone());
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

/// Desired media session transition requested by a GB28181 driver event.
enum MediaSessionTransition {
    /// Progress the session to Active.
    Start,
    /// Tear the session down.
    Stop,
    /// Fail the session with the given reason.
    Fail(String),
}

/// Persists a single [`MediaSession`] state transition and appends the resulting
/// `MediaSessionStateChanged` event to the outbox in the same UnitOfWork.
async fn save_and_append_media_session_transition(
    uow: &mut dyn UnitOfWork,
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    media_session_id: MediaSessionId,
    session: &MediaSession,
    event: DomainEvent,
) -> Result<(), SignalError> {
    uow.media_session_repository().save(session).await?;
    let aggregate_ref = ResourceRef {
        tenant_id,
        kind: ResourceKind::MediaSession,
        id: ResourceId::MediaSession(media_session_id),
    };
    uow.outbox()
        .append(Event::new(
            state.id_generator.as_ref(),
            state.clock.as_ref(),
            context,
            tenant_id,
            aggregate_ref,
            session.revision().0,
            event,
        ))
        .await?;
    Ok(())
}

/// Drives a [`MediaSession`] through the requested transition, saving and
/// appending each resulting `MediaSessionStateChanged` event one step at a time
/// so the repository's optimistic-concurrency check succeeds. The
/// `Gb28181EventReceived` envelope is always appended in the same UnitOfWork.
#[allow(clippy::too_many_arguments)]
async fn handle_media_session_event(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    media_session_id: MediaSessionId,
    transition: MediaSessionTransition,
    device_id: Option<DeviceId>,
    external_id: Option<&str>,
    event_type: &str,
    payload: BTreeMap<String, String>,
) -> Result<(), SignalError> {
    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let session = uow
        .media_session_repository()
        .get(tenant_id, media_session_id)
        .await?;

    if let Some(mut session) = session {
        let clock = state.clock.as_ref();

        match transition {
            MediaSessionTransition::Start => match session.state() {
                MediaSessionState::Requested => {
                    let event = session.allocating(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                    let event = session.inviting(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                    let event = session.active(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                MediaSessionState::Allocating => {
                    let event = session.inviting(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                    let event = session.active(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                MediaSessionState::Inviting => {
                    let event = session.active(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                _ => {}
            },
            MediaSessionTransition::Stop => match session.state() {
                MediaSessionState::Active => {
                    let event = session.stop(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                    if session.state() == MediaSessionState::Stopping {
                        let event = session.stopped(clock)?;
                        save_and_append_media_session_transition(
                            &mut *uow,
                            state,
                            context,
                            tenant_id,
                            media_session_id,
                            &session,
                            event,
                        )
                        .await?;
                    }
                }
                MediaSessionState::Stopping => {
                    let event = session.stopped(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                MediaSessionState::Requested
                | MediaSessionState::Allocating
                | MediaSessionState::Inviting => {
                    let event = session.stop(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                _ => {}
            },
            MediaSessionTransition::Fail(reason) => {
                if !session.state().is_terminal() {
                    let event = session.failed(MediaSessionError::new("gb28181", reason), clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
            }
        };
    }

    // Always append the GB28181 envelope so the driver event is recorded even
    // when the session is unknown or already in a terminal state.
    let envelope = build_gb_event(
        state,
        context,
        tenant_id,
        device_id,
        external_id,
        event_type,
        payload,
    );
    uow.outbox().append(envelope).await?;
    uow.commit().await?;
    Ok(())
}

/// Builds a [`DomainEvent::Gb28181EventReceived`] outbox event.
///
/// When an internal device identifier is known the event is attached to the
/// device aggregate; otherwise it is attached to a synthetic event aggregate.
fn build_gb_event(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: Option<DeviceId>,
    external_id: Option<&str>,
    event_type: &str,
    payload: BTreeMap<String, String>,
) -> Event<DomainEvent> {
    let event_id = state.id_generator.generate_event_id();
    let aggregate_ref = match device_id {
        Some(id) => ResourceRef {
            tenant_id,
            kind: ResourceKind::Device,
            id: ResourceId::Device(id),
        },
        None => ResourceRef {
            tenant_id,
            kind: ResourceKind::Event,
            id: ResourceId::Event(event_id),
        },
    };
    Event {
        event_id,
        tenant_id,
        aggregate_ref,
        aggregate_sequence: 0,
        occurred_at: state.clock.now_wall(),
        correlation_id: context.correlation_id,
        causation_id: context.message_id,
        traceparent: context.traceparent.clone(),
        tracestate: context.tracestate.clone(),
        source: context.node_id.unwrap_or_default(),
        payload: DomainEvent::Gb28181EventReceived {
            tenant_id,
            device_id,
            event_type: event_type.to_string(),
            protocol: Protocol::Gb28181,
            external_id: external_id.map(String::from),
            payload,
        },
    }
}

/// Appends a [`DomainEvent::Gb28181EventReceived`] to the outbox.
async fn append_gb_event(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: Option<DeviceId>,
    external_id: Option<&str>,
    event_type: &str,
    payload: BTreeMap<String, String>,
) -> Result<(), SignalError> {
    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let event = build_gb_event(
        state,
        context,
        tenant_id,
        device_id,
        external_id,
        event_type,
        payload,
    );
    uow.outbox().append(event).await?;
    uow.commit().await?;
    Ok(())
}

/// Serializes a slice of GB28181 record items to a JSON array.
fn serialize_record_items(records: &[GbRecordItem]) -> Option<String> {
    let maps: Vec<BTreeMap<String, String>> = records
        .iter()
        .map(|r| {
            let mut m = BTreeMap::new();
            m.insert("device_id".to_string(), r.device_id.clone());
            if let Some(v) = &r.name {
                m.insert("name".to_string(), v.clone());
            }
            if let Some(v) = &r.file_path {
                m.insert("file_path".to_string(), v.clone());
            }
            if let Some(v) = &r.start_time {
                m.insert("start_time".to_string(), v.clone());
            }
            if let Some(v) = &r.end_time {
                m.insert("end_time".to_string(), v.clone());
            }
            if let Some(v) = &r.secrecy {
                m.insert("secrecy".to_string(), v.clone());
            }
            if let Some(v) = &r.record_type {
                m.insert("record_type".to_string(), v.clone());
            }
            if let Some(v) = &r.recorder_id {
                m.insert("recorder_id".to_string(), v.clone());
            }
            if let Some(v) = &r.file_size {
                m.insert("file_size".to_string(), v.clone());
            }
            m
        })
        .collect();
    serde_json::to_string(&maps).ok()
}

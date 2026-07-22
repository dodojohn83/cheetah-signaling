//! GB28181 event processing helpers.
//!
//! Device presence, catalog replacement and outbox helpers used by the event
//! processing pipeline.

use cheetah_domain::{Connectivity, Device, DomainEvent, Protocol};
use cheetah_gb28181_module::DeviceId as GbDeviceId;
use cheetah_gb28181_module::bootstrap;
use cheetah_gb28181_module::xml::CatalogItem as GbCatalogItem;
use cheetah_http_api::state::ApiState;
use cheetah_signal_application::{
    ChannelDescriptor, MarkDeviceOfflineRequest, MarkDeviceOnlineRequest, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest, SubmitOperationRequest, UpdateDeviceCapabilitiesRequest,
};
use cheetah_signal_types::{
    Deadline, DeviceId, DurationMs, Event, OwnerEpoch, ProtocolIdentity, RequestContext,
    ResourceId, ResourceKind, ResourceRef, SignalError, SignalErrorKind, TenantId,
};
use cheetah_storage_api::StorageError;
use std::collections::BTreeMap;
use tracing::warn;

pub(crate) fn storage_error(e: StorageError) -> SignalError {
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

pub(crate) async fn resolve_device_id(
    state: &ApiState,
    tenant_id: TenantId,
    external_id: &str,
) -> Option<DeviceId> {
    resolve_device(state, tenant_id, external_id)
        .await
        .map(|d| d.device_id())
}

pub(crate) async fn ensure_online(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: &GbDeviceId,
    force: bool,
) -> Result<Option<DeviceId>, SignalError> {
    let external_id = device_id.as_ref();
    if let Some(device) = resolve_device(state, tenant_id, external_id).await {
        let internal_id = device.device_id();
        if !matches!(device.connectivity(), Connectivity::Online) {
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

pub(crate) async fn mark_offline(
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
pub(crate) async fn submit_bootstrap_queries(
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

pub(crate) async fn update_device_info(
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

pub(crate) async fn replace_catalog(
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

/// Builds a [`DomainEvent::Gb28181EventReceived`] outbox event.
///
/// When an internal device identifier is known the event is attached to the
/// device aggregate; otherwise it is attached to a synthetic event aggregate.
pub(crate) fn build_gb_event(
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
pub(crate) async fn append_gb_event(
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

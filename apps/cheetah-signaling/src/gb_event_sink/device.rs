//! Device resolution, presence and bootstrap-query helpers.

use cheetah_domain::{Connectivity, Device, Protocol};
use cheetah_gb28181_module::DeviceId as GbDeviceId;
use cheetah_gb28181_module::bootstrap;
use cheetah_http_api::state::ApiState;
use cheetah_signal_application::{
    MarkDeviceOfflineRequest, MarkDeviceOnlineRequest, RegisterDeviceRequest,
    SubmitOperationRequest, UpdateDeviceCapabilitiesRequest,
};
use cheetah_signal_types::{
    Deadline, DeviceId, DurationMs, OwnerEpoch, ProtocolIdentity, RequestContext, ResourceId,
    ResourceKind, ResourceRef, SignalError, TenantId,
};
use std::collections::BTreeMap;
use tracing::warn;

use super::storage_error;

pub(super) async fn resolve_device(
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

pub(super) async fn resolve_device_id(
    state: &ApiState,
    tenant_id: TenantId,
    external_id: &str,
) -> Option<DeviceId> {
    resolve_device(state, tenant_id, external_id)
        .await
        .map(|d| d.device_id())
}

pub(super) async fn ensure_online(
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

pub(super) async fn mark_offline(
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
pub(super) async fn submit_bootstrap_queries(
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

pub(super) async fn update_device_info(
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

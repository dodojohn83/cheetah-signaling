//! Device HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use cheetah_domain::DomainError;
use cheetah_signal_application::dto::{
    RegisterDeviceRequest, RegisterDeviceResult, RetireDeviceRequest,
    UpdateDeviceCapabilitiesRequest,
};
use cheetah_signal_types::{AuditOutcome, DeviceId, Page, UtcTimestamp};
use std::sync::Arc;

pub async fn list_devices(
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<cheetah_signal_application::dto::DeviceDto>>, HttpError> {
    ctx.require_scope("viewer")?;
    let page = query.page_request()?;
    let updated_after = query
        .updated_after
        .as_deref()
        .map(UtcTimestamp::parse_rfc3339)
        .transpose()
        .map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = uow
        .device_repository()
        .list(
            ctx.tenant_id,
            query.protocol,
            query.status,
            query.name_prefix,
            updated_after,
            page,
        )
        .await
        .map_err(HttpError::from)?;
    Ok(Json(result.map(|d| {
        cheetah_signal_application::dto::DeviceDto::from(&d)
    })))
}

pub async fn create_device(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(request): Json<RegisterDeviceRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), HttpError> {
    ctx.require_scope("operator")?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let RegisterDeviceResult { device, created } = state
        .device_service
        .register_or_update_device(&ctx.0, &mut *uow, request)
        .await
        .map_err(HttpError::from)?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let target_id = Some(device.device_id.to_string());
    let action = if created {
        "device.create"
    } else {
        "device.update"
    };
    let body = serde_json::to_value(device).map_err(HttpError::from)?;
    crate::audit::record(
        &state,
        &ctx,
        action,
        "device",
        target_id,
        None,
        AuditOutcome::Success,
    );
    Ok((status, Json(body)))
}

pub async fn get_device(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("viewer")?;
    let device_id = id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let device = uow
        .device_repository()
        .get(ctx.tenant_id, device_id)
        .await
        .map_err(HttpError::from)?
        .ok_or_else(|| HttpError::from(DomainError::not_found("device", device_id.to_string())))?;
    Ok(Json(
        serde_json::to_value(cheetah_signal_application::dto::DeviceDto::from(&device))
            .map_err(HttpError::from)?,
    ))
}

pub async fn update_device(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(request): Json<UpdateDeviceCapabilitiesRequest>,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("operator")?;
    let device_id = id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = state
        .device_service
        .update_device_capabilities(&ctx.0, &mut *uow, device_id, request)
        .await
        .map_err(HttpError::from)?;
    let target_id = Some(result.device_id.to_string());
    let body = serde_json::to_value(result).map_err(HttpError::from)?;
    crate::audit::record(
        &state,
        &ctx,
        "device.update_capabilities",
        "device",
        target_id,
        None,
        AuditOutcome::Success,
    );
    Ok(Json(body))
}

pub async fn retire_device(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("operator")?;
    let device_id = id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = state
        .device_service
        .retire_device(&ctx.0, &mut *uow, device_id, RetireDeviceRequest {})
        .await
        .map_err(HttpError::from)?;
    let target_id = Some(result.device_id.to_string());
    let body = serde_json::to_value(result).map_err(HttpError::from)?;
    crate::audit::record(
        &state,
        &ctx,
        "device.delete",
        "device",
        target_id,
        None,
        AuditOutcome::Success,
    );
    Ok(Json(body))
}

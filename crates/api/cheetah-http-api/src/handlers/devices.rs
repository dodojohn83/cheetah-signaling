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
    RegisterDeviceRequest, RetireDeviceRequest, UpdateDeviceCapabilitiesRequest,
};
use cheetah_signal_types::{DeviceId, Page};
use std::sync::Arc;

pub async fn list_devices(
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    ctx.require_scope("viewer")?;
    Ok(Json(Page::new(Vec::new())))
}

pub async fn create_device(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(request): Json<RegisterDeviceRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), HttpError> {
    ctx.require_scope("operator")?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = state
        .device_service
        .register_or_update_device(&ctx.0, &mut *uow, request)
        .await
        .map_err(HttpError::from)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(result).map_err(HttpError::from)?),
    ))
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
    Ok(Json(serde_json::to_value(result).map_err(HttpError::from)?))
}

pub async fn retire_device(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(_request): Json<RetireDeviceRequest>,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("operator")?;
    let device_id = id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = state
        .device_service
        .retire_device(&ctx.0, &mut *uow, device_id, RetireDeviceRequest {})
        .await
        .map_err(HttpError::from)?;
    Ok(Json(serde_json::to_value(result).map_err(HttpError::from)?))
}

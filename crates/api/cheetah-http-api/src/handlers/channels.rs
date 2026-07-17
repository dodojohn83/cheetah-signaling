//! Channel HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use cheetah_signal_application::dto::ReplaceChannelCatalogRequest;
use cheetah_signal_types::{AuditOutcome, DeviceId, Page, UtcTimestamp};
use std::sync::Arc;

pub async fn list_channels(
    Path(device_id): Path<String>,
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<cheetah_signal_application::dto::ChannelDto>>, HttpError> {
    ctx.require_scope("viewer")?;
    let device_id = device_id.parse::<DeviceId>().map_err(HttpError::from)?;
    let page = query.page_request()?;
    let updated_after = query
        .updated_after
        .as_deref()
        .map(UtcTimestamp::parse_rfc3339)
        .transpose()
        .map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = uow
        .channel_repository()
        .list(
            ctx.tenant_id,
            device_id,
            query.status,
            query.name_prefix,
            updated_after,
            page,
        )
        .await
        .map_err(HttpError::from)?;
    Ok(Json(result.map(|c| {
        cheetah_signal_application::dto::ChannelDto::from(&c)
    })))
}

pub async fn replace_catalog(
    Path(device_id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(request): Json<ReplaceChannelCatalogRequest>,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    let device_id = device_id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = state
        .device_service
        .replace_channel_catalog(&ctx.0, &mut *uow, device_id, request)
        .await
        .map_err(HttpError::from)?;
    let target_id = Some(result.device_id.to_string());
    let body = serde_json::to_value(result).map_err(HttpError::from)?;
    crate::audit::record(
        &state,
        &ctx,
        "device.channel_catalog.replace",
        "device",
        target_id,
        None,
        AuditOutcome::Success,
    );
    Ok(Json(body))
}

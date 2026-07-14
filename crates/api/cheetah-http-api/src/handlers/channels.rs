//! Channel HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use cheetah_signal_application::dto::ReplaceChannelCatalogRequest;
use cheetah_signal_types::{DeviceId, Page};
use std::sync::Arc;

pub async fn list_channels(
    Path(_device_id): Path<String>,
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    ctx.require_scope("viewer")?;
    Err(HttpError::NotImplemented(
        "channel list pagination is not yet implemented".to_string(),
    ))
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
    Ok(Json(serde_json::to_value(result).map_err(HttpError::from)?))
}

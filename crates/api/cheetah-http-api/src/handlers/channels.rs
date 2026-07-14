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
    _ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    Ok(Json(Page::new(Vec::new())))
}

pub async fn replace_catalog(
    Path(device_id): Path<String>,
    State(state): State<Arc<ApiState>>,
    _ctx: ApiRequestContext,
    Json(request): Json<ReplaceChannelCatalogRequest>,
) -> Result<impl IntoResponse, HttpError> {
    let device_id = device_id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = state
        .device_service
        .replace_channel_catalog(&_ctx.0, &mut *uow, device_id, request)
        .await
        .map_err(HttpError::from)?;
    uow.commit().await.map_err(HttpError::from)?;
    Ok(Json(serde_json::to_value(result).map_err(HttpError::from)?))
}

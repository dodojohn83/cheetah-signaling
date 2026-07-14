//! Webhook HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::Arc;

use crate::ListQuery;

pub async fn list_webhooks(
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("viewer")?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"message": "webhooks not implemented"})),
    ))
}

pub async fn create_webhook(
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(_body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"message": "webhooks not implemented"})),
    ))
}

pub async fn get_webhook(
    Path(_id): Path<String>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("viewer")?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"message": "webhooks not implemented"})),
    ))
}

pub async fn list_deliveries(
    Path(_id): Path<String>,
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("viewer")?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"message": "webhooks not implemented"})),
    ))
}

pub async fn trigger_delivery(
    Path(_id): Path<String>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(_body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"message": "webhooks not implemented"})),
    ))
}

pub async fn replay_delivery(
    Path((_webhook_id, _delivery_id)): Path<(String, String)>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"message": "webhooks not implemented"})),
    ))
}

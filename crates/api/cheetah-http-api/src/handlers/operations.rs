//! Operation HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use cheetah_signal_types::{OperationId, Page};
use std::sync::Arc;

pub async fn list_operations(
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    ctx.require_scope("viewer")?;
    Err(HttpError::NotImplemented(
        "operation list pagination is not yet implemented".to_string(),
    ))
}

pub async fn create_operation(
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(_request): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), HttpError> {
    ctx.require_scope("operator")?;
    Err(HttpError::NotImplemented(
        "generic operation submission is not implemented".to_string(),
    ))
}

pub async fn get_operation(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("viewer")?;
    let operation_id = id.parse::<OperationId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let operation = state
        .operation_service
        .get_operation(&mut *uow, ctx.tenant_id, operation_id)
        .await
        .map_err(HttpError::from)?;
    Ok(Json(
        serde_json::to_value(operation).map_err(HttpError::from)?,
    ))
}

pub async fn cancel_operation(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("operator")?;
    let operation_id = id.parse::<OperationId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let operation = state
        .operation_service
        .cancel_operation(&ctx.0, &mut *uow, operation_id)
        .await
        .map_err(HttpError::from)?;
    Ok(Json(
        serde_json::to_value(operation).map_err(HttpError::from)?,
    ))
}

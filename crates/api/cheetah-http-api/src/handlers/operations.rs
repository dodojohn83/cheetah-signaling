//! Operation HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, IdempotencyKey, JsonBody, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use cheetah_signal_application::dto::{OperationDto, SubmitOperationRequest};
use cheetah_signal_types::{AuditOutcome, DeviceId, OperationId, Page, UtcTimestamp};
use std::sync::Arc;

pub async fn list_operations(
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<OperationDto>>, HttpError> {
    ctx.require_scope("viewer")?;
    let page = query.page_request()?;
    let device_id = query
        .device_id
        .as_deref()
        .map(str::parse::<DeviceId>)
        .transpose()
        .map_err(HttpError::from)?;
    let updated_after = query
        .updated_after
        .as_deref()
        .map(UtcTimestamp::parse_rfc3339)
        .transpose()
        .map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = uow
        .operation_repository()
        .list(ctx.tenant_id, device_id, query.status, updated_after, page)
        .await
        .map_err(HttpError::from)?;
    Ok(Json(result.map(|o| OperationDto::from(&o))))
}

pub async fn create_operation(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    JsonBody(mut request): JsonBody<SubmitOperationRequest>,
) -> Result<axum::response::Response, HttpError> {
    ctx.require_scope("operator")?;
    // Prefer the mandatory Idempotency-Key header as the authoritative key.
    request.idempotency_key = idempotency.0;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let operation = state
        .operation_service
        .submit_operation(&ctx.0, &mut *uow, request)
        .await
        .map_err(HttpError::from)?;
    let operation_id = operation.operation_id.to_string();
    crate::audit::record(
        &state,
        &ctx,
        "operation.create",
        "operation",
        Some(operation_id.clone()),
        None,
        AuditOutcome::Success,
    );
    let mut response = (StatusCode::ACCEPTED, Json(operation)).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("/api/v1/operations/{operation_id}")) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    Ok(response)
}

pub async fn get_operation(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<OperationDto>, HttpError> {
    ctx.require_scope("viewer")?;
    let operation_id = id.parse::<OperationId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let operation = state
        .operation_service
        .get_operation(&mut *uow, ctx.tenant_id, operation_id)
        .await
        .map_err(HttpError::from)?;
    Ok(Json(operation))
}

pub async fn cancel_operation(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<OperationDto>, HttpError> {
    ctx.require_scope("operator")?;
    let operation_id = id.parse::<OperationId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let operation = state
        .operation_service
        .cancel_operation(&ctx.0, &mut *uow, operation_id)
        .await
        .map_err(HttpError::from)?;
    crate::audit::record(
        &state,
        &ctx,
        "operation.cancel",
        "operation",
        Some(operation_id.to_string()),
        None,
        AuditOutcome::Success,
    );
    Ok(Json(operation))
}

//! Webhook HTTP handlers.

use crate::{ApiRequestContext, ApiState, HttpError, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use cheetah_signal_application::{
    CreateWebhookRequest, TriggerWebhookRequest, UpdateWebhookRequest,
};
use cheetah_signal_types::{DeliveryId, WebhookId};
use std::sync::Arc;

fn parse_webhook_id(id: &str) -> Result<WebhookId, HttpError> {
    id.parse::<WebhookId>().map_err(HttpError::Signal)
}

fn parse_delivery_id(id: &str) -> Result<DeliveryId, HttpError> {
    id.parse::<DeliveryId>().map_err(HttpError::Signal)
}

/// Lists webhook configurations for the tenant.
pub async fn list_webhooks(
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("viewer")?;
    let page = query.page_request()?;
    let result = state
        .webhook_service()?
        .list_webhooks(&ctx, query.enabled, query.event_type, page)
        .await?;
    Ok((StatusCode::OK, Json(result)))
}

/// Creates a new webhook configuration.
pub async fn create_webhook(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(body): Json<CreateWebhookRequest>,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    let config = state.webhook_service()?.create_webhook(&ctx, body).await?;
    Ok((StatusCode::CREATED, Json(config)))
}

/// Gets a webhook configuration by id.
pub async fn get_webhook(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("viewer")?;
    let webhook_id = parse_webhook_id(&id)?;
    let config = state
        .webhook_service()?
        .get_webhook(&ctx, webhook_id)
        .await?;
    Ok((StatusCode::OK, Json(config)))
}

/// Updates a webhook configuration.
pub async fn update_webhook(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(body): Json<UpdateWebhookRequest>,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    let webhook_id = parse_webhook_id(&id)?;
    let config = state
        .webhook_service()?
        .update_webhook(&ctx, webhook_id, body)
        .await?;
    Ok((StatusCode::OK, Json(config)))
}

/// Deletes a webhook configuration.
pub async fn delete_webhook(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    let webhook_id = parse_webhook_id(&id)?;
    state
        .webhook_service()?
        .delete_webhook(&ctx, webhook_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lists deliveries for a webhook.
pub async fn list_deliveries(
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("viewer")?;
    let webhook_id = parse_webhook_id(&id)?;
    let page = query.page_request()?;
    let result = state
        .webhook_service()?
        .list_deliveries(&ctx, webhook_id, query.status, page)
        .await?;
    Ok((StatusCode::OK, Json(result)))
}

/// Manually triggers a test delivery for a webhook.
pub async fn trigger_delivery(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(body): Json<TriggerWebhookRequest>,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    let webhook_id = parse_webhook_id(&id)?;
    let delivery = state
        .webhook_service()?
        .trigger_webhook(&ctx, webhook_id, body)
        .await?;
    Ok((StatusCode::CREATED, Json(delivery)))
}

/// Replays a previous delivery as a new pending delivery.
pub async fn replay_delivery(
    Path((webhook_id, delivery_id)): Path<(String, String)>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("operator")?;
    let webhook_id = parse_webhook_id(&webhook_id)?;
    let delivery_id = parse_delivery_id(&delivery_id)?;
    let delivery = state
        .webhook_service()?
        .replay_delivery(&ctx, webhook_id, delivery_id)
        .await?;
    Ok((StatusCode::CREATED, Json(delivery)))
}

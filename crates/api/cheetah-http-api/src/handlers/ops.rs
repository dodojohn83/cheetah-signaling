//! Administrative operations handlers.

use crate::{ApiRequestContext, ApiState, HttpError};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use cheetah_signal_application::dto::{DeviceDto, MediaSessionDto, OperationDto};
use cheetah_signal_types::{DeviceId, PageRequest, SignalConfig, SignalError, SignalErrorKind};
use std::sync::Arc;

/// Validates a submitted configuration without applying it.
pub async fn validate_config(
    ctx: ApiRequestContext,
    Json(config): Json<SignalConfig>,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("system_admin")?;
    config.validate().map_err(HttpError::Signal)?;
    Ok((StatusCode::OK, Json(serde_json::json!({"valid": true}))))
}

/// Returns the current database migration status.
pub async fn db_status(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("system_admin")?;
    let info = state
        .storage
        .migration()
        .status()
        .await
        .map_err(HttpError::from)?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "last_applied": info.last_applied,
            "latest_known": info.latest_known,
            "status": migration_status_string(&info.status),
        })),
    ))
}

/// Runs pending database migrations.
pub async fn db_migrate(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("system_admin")?;
    state
        .storage
        .migration()
        .run()
        .await
        .map_err(HttpError::from)?;
    db_status(State(state), ctx).await
}

/// Requests a graceful node drain by cancelling background work tokens.
pub async fn node_drain(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("system_admin")?;
    state.cancel.cancel();
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"draining": true})),
    ))
}

/// Returns a sanitized diagnostics package for a device.
pub async fn device_diagnostics(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("system_admin")?;
    let device_id = id.parse::<DeviceId>().map_err(HttpError::from)?;
    let tenant_id = ctx.tenant_id;

    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let device = uow
        .device_repository()
        .get(tenant_id, device_id)
        .await
        .map_err(HttpError::from)?
        .ok_or_else(|| {
            HttpError::Signal(SignalError::new(
                SignalErrorKind::NotFound,
                "device not found",
            ))
        })?;

    let page = PageRequest::new(100).map_err(HttpError::Signal)?;
    let channels = uow
        .channel_repository()
        .list(tenant_id, device_id, None, None, None, page.clone())
        .await
        .map_err(HttpError::from)?;
    let operations = uow
        .operation_repository()
        .list(tenant_id, Some(device_id), None, None, page.clone())
        .await
        .map_err(HttpError::from)?;
    let sessions = uow
        .media_session_repository()
        .list(tenant_id, Some(device_id), None, None, None, page)
        .await
        .map_err(HttpError::from)?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "device": DeviceDto::from(&device),
            "channel_count": channels.items.len(),
            "channels": channels.items.iter().take(20).map(|c| c.name().to_string()).collect::<Vec<_>>(),
            "operation_count": operations.items.len(),
            "operations": operations.items.iter().take(20).map(OperationDto::from).collect::<Vec<_>>(),
            "media_session_count": sessions.items.len(),
            "media_sessions": sessions.items.iter().take(20).map(MediaSessionDto::from).collect::<Vec<_>>(),
        })),
    ))
}

/// Replays pending outbox events to the event bus.
pub async fn outbox_replay(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<impl IntoResponse, HttpError> {
    ctx.require_scope("system_admin")?;
    let limit = 1000usize;
    let now = state.clock.now_wall();

    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let pending = uow
        .outbox()
        .pending(now, limit)
        .await
        .map_err(HttpError::from)?;
    let mut published = 0u32;

    for entry in pending {
        let envelope = cheetah_message_api::encode_event(&entry.event)
            .map_err(|e| HttpError::Internal(e.to_string()))?;
        let subject = cheetah_message_api::event_subject(entry.event.tenant_id, "domain_event");
        state
            .event_bus
            .publish(&subject, &envelope)
            .await
            .map_err(|e| HttpError::Internal(e.to_string()))?;
        uow.outbox()
            .mark_published(entry.event.event_id)
            .await
            .map_err(HttpError::from)?;
        published += 1;
    }

    uow.commit().await.map_err(HttpError::from)?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"replayed": published})),
    ))
}

/// Triggers background reconciliation.
pub async fn reconcile(
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<StatusCode, HttpError> {
    ctx.require_scope("system_admin")?;
    Err(HttpError::NotImplemented(
        "reconciliation trigger is not yet wired to background reconcilers".to_string(),
    ))
}

fn migration_status_string(status: &cheetah_storage_api::MigrationStatus) -> String {
    match status {
        cheetah_storage_api::MigrationStatus::Empty => "empty".to_string(),
        cheetah_storage_api::MigrationStatus::Current => "current".to_string(),
        cheetah_storage_api::MigrationStatus::Behind { current, target } => {
            format!("behind:{current}/{target}")
        }
        cheetah_storage_api::MigrationStatus::Diverged { applied, known } => {
            format!("diverged:{applied}/{known}")
        }
        _ => "unknown".to_string(),
    }
}

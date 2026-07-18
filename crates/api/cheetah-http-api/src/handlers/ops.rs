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

    let page = PageRequest::new(1000).map_err(HttpError::Signal)?;
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

    let channel_sample: Vec<_> = channels
        .items
        .iter()
        .take(20)
        .map(|c| c.name().to_string())
        .collect();
    let operation_sample: Vec<_> = operations
        .items
        .iter()
        .take(20)
        .map(OperationDto::from)
        .collect();
    let media_session_sample: Vec<_> = sessions
        .items
        .iter()
        .take(20)
        .map(MediaSessionDto::from)
        .collect();

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "device": DeviceDto::from(&device),
            "channel_sample_count": channel_sample.len(),
            "channel_total": channels.total,
            "channel_next_cursor": channels.next_cursor,
            "channel_sample": channel_sample,
            "operation_sample_count": operation_sample.len(),
            "operation_total": operations.total,
            "operation_next_cursor": operations.next_cursor,
            "operation_sample": operation_sample,
            "media_session_sample_count": media_session_sample.len(),
            "media_session_total": sessions.total,
            "media_session_next_cursor": sessions.next_cursor,
            "media_session_sample": media_session_sample,
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
    let service = cheetah_signal_application::EventService::new();

    // Read pending entries in a short transaction; do not hold it across bus I/O.
    let mut read_uow = state.storage.begin().await.map_err(HttpError::from)?;
    let pending = service
        .read_pending(read_uow.outbox(), now, limit)
        .await
        .map_err(HttpError::from)?;
    read_uow.commit().await.map_err(HttpError::from)?;

    if pending.is_empty() {
        return Ok((StatusCode::OK, Json(serde_json::json!({"replayed": 0}))));
    }

    let publisher = cheetah_message_api::RawEventBusPublisher::new(state.event_bus.as_ref());
    let results = service.publish_events(&publisher, &pending).await;

    // Record publish outcomes in a separate transaction.
    let mut write_uow = state.storage.begin().await.map_err(HttpError::from)?;
    let published = service
        .record_results(write_uow.outbox(), now, &pending, &results)
        .await
        .map_err(HttpError::from)?;
    write_uow.commit().await.map_err(HttpError::from)?;

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

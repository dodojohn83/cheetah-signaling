//! Health, readiness and metrics handlers.

use crate::{ApiState, AuthContext, HttpError};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use cheetah_storage_api::MigrationStatus;
use std::sync::Arc;

/// Liveness probe.
pub async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

/// Readiness probe: verifies storage migration status and that the node is not draining.
pub async fn ready(State(state): State<Arc<ApiState>>) -> Result<impl IntoResponse, HttpError> {
    if state.cancel.is_cancelled() {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "draining"})),
        ));
    }
    let migration = state
        .storage
        .migration()
        .status()
        .await
        .map_err(HttpError::from)?;
    if migration.status != MigrationStatus::Current {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "not_ready", "reason": "migration"})),
        ));
    }
    if state.config.media_nodes_required {
        let now = state.clock.now_wall();
        let page = cheetah_signal_types::PageRequest::new(1).map_err(HttpError::from)?;
        let alive = state
            .storage
            .media_node_repository()
            .list_alive(now, page)
            .await
            .map_err(HttpError::from)?;
        if alive.items.is_empty() {
            return Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "not_ready",
                    "reason": "media_nodes_unavailable"
                })),
            ));
        }
    }
    Ok((StatusCode::OK, Json(serde_json::json!({"status": "ready"}))))
}

/// Prometheus metrics exposition.
pub async fn metrics(
    State(state): State<Arc<ApiState>>,
    auth: AuthContext,
) -> Result<Response, HttpError> {
    auth.require_scope("viewer")?;
    Ok(crate::metrics::metrics_response(
        state.metrics.clone(),
        state.media_metrics.clone(),
    ))
}

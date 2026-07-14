//! Health, readiness and metrics handlers.

use crate::{ApiState, HttpError};
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

/// Readiness probe: verifies storage migration status.
pub async fn ready(State(state): State<Arc<ApiState>>) -> Result<impl IntoResponse, HttpError> {
    let migration = state
        .storage
        .migration()
        .status()
        .await
        .map_err(HttpError::from)?;
    if migration.status != MigrationStatus::Current {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "not_ready"})),
        ));
    }
    Ok((StatusCode::OK, Json(serde_json::json!({"status": "ready"}))))
}

/// Prometheus metrics exposition.
pub async fn metrics(State(state): State<Arc<ApiState>>) -> Response {
    crate::metrics::metrics_response(state.metrics.clone())
}

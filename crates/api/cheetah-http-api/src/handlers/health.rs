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

/// Liveness probe alias (`/healthz`).
///
/// Liveness only asserts the process and its critical workers are running; it
/// deliberately does not consult dependencies or runtime pressure so that a
/// degraded-but-alive node is not killed by an orchestrator.
pub async fn healthz() -> impl IntoResponse {
    live().await
}

/// Readiness probe: verifies storage migration status and that the node is not draining.
pub async fn ready(State(state): State<Arc<ApiState>>) -> Result<impl IntoResponse, HttpError> {
    if let Some(unavailable) = dependency_check(&state).await? {
        return Ok(unavailable);
    }
    Ok((StatusCode::OK, Json(serde_json::json!({"status": "ready"}))))
}

/// Readiness probe alias (`/readyz`) that additionally reports GB28181 runtime
/// degradation from queue saturation and timer lag.
///
/// A single failed dependency yields not-ready. When dependencies are healthy
/// the runtime health source is consulted: sustained mailbox saturation or
/// critical timer lag makes the node not-ready, while lesser pressure is
/// surfaced as a `degraded` flag on an otherwise-ready response. The body is a
/// bounded summary and never lists device or session identifiers.
pub async fn readyz(State(state): State<Arc<ApiState>>) -> Result<Response, HttpError> {
    if let Some(unavailable) = dependency_check(&state).await? {
        return Ok(unavailable.into_response());
    }

    let Some(runtime_health) = state.runtime_health.as_ref() else {
        return Ok((StatusCode::OK, Json(serde_json::json!({"status": "ready"}))).into_response());
    };

    let health = runtime_health.runtime_health();
    let reasons: Vec<&str> = health.reasons.iter().map(|r| r.as_str()).collect();
    let body = serde_json::json!({
        "status": if health.ready { "ready" } else { "not_ready" },
        "degraded": health.degraded,
        "reasons": reasons,
        "runtime": {
            "max_shard_mailbox_depth": health.max_shard_mailbox_depth,
            "active_actors": health.active_actors,
            "timer_lag_ms": health.timer_lag_ms,
        }
    });
    let status = if health.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Ok((status, Json(body)).into_response())
}

/// Runs the dependency readiness checks shared by `/health/ready` and `/readyz`.
///
/// Returns `Some(response)` when a dependency is unavailable and the node must
/// report not-ready, or `None` when all dependencies are satisfied.
async fn dependency_check(
    state: &ApiState,
) -> Result<Option<(StatusCode, Json<serde_json::Value>)>, HttpError> {
    if state.cancel.is_cancelled() {
        return Ok(Some((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "draining"})),
        )));
    }
    let migration = state
        .storage
        .migration()
        .status()
        .await
        .map_err(HttpError::from)?;
    if migration.status != MigrationStatus::Current {
        return Ok(Some((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "not_ready", "reason": "migration"})),
        )));
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
            return Ok(Some((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "not_ready",
                    "reason": "media_nodes_unavailable"
                })),
            )));
        }
    }
    Ok(None)
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
        state.gb_metrics.clone(),
    ))
}

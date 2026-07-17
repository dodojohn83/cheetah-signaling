//! Axum router setup and middleware.

use crate::ApiState;
use crate::handlers::{
    channels, devices, events, health, media, nodes, operations, tenants, webhooks,
};
use crate::rate_limit::rate_limit_middleware;
use crate::request_limits::request_limits_middleware;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, StatusCode},
    middleware::from_fn_with_state,
    response::Json,
    routing::{delete, get, patch, post},
};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, Any, CorsLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

/// Builds the public API router.
pub fn build_router(state: ApiState) -> Router {
    let timeout = Duration::from_millis(state.config.read_timeout_ms);
    let body_limit = state.config.request_body_limit_bytes;
    let cors = build_cors_layer(&state.config.cors_allowed_origins);
    let shared_state = Arc::new(state);
    let api = Router::new()
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready))
        .route("/metrics", get(health::metrics))
        .route("/api/v1/tenants", get(tenants::list_tenants))
        .route("/api/v1/tenants", post(tenants::create_tenant))
        .route("/api/v1/devices", get(devices::list_devices))
        .route("/api/v1/devices", post(devices::create_device))
        .route("/api/v1/devices/{id}", get(devices::get_device))
        .route("/api/v1/devices/{id}", patch(devices::update_device))
        .route("/api/v1/devices/{id}", delete(devices::retire_device))
        .route(
            "/api/v1/devices/{id}/channels",
            get(channels::list_channels),
        )
        .route(
            "/api/v1/devices/{id}/channels/catalog",
            post(channels::replace_catalog),
        )
        .route("/api/v1/operations", get(operations::list_operations))
        .route("/api/v1/operations", post(operations::create_operation))
        .route("/api/v1/operations/{id}", get(operations::get_operation))
        .route(
            "/api/v1/operations/{id}/cancel",
            post(operations::cancel_operation),
        )
        .route("/api/v1/media/sessions", get(media::list_sessions))
        .route("/api/v1/media/sessions", post(media::create_session))
        .route("/api/v1/media/sessions/{id}", get(media::get_session))
        .route(
            "/api/v1/media/sessions/{id}/stop",
            post(media::stop_session),
        )
        .route(
            "/api/v1/media/sessions/{id}/control",
            post(media::control_session),
        )
        .route("/api/v1/nodes", get(nodes::list_nodes))
        .route("/api/v1/media-nodes", get(nodes::list_media_nodes))
        .route("/api/v1/events/stream", get(events::event_stream))
        .route("/api/v1/webhooks", get(webhooks::list_webhooks))
        .route("/api/v1/webhooks", post(webhooks::create_webhook))
        .route("/api/v1/webhooks/{id}", get(webhooks::get_webhook))
        .route("/api/v1/webhooks/{id}", patch(webhooks::update_webhook))
        .route("/api/v1/webhooks/{id}", delete(webhooks::delete_webhook))
        .route(
            "/api/v1/webhooks/{id}/deliveries",
            get(webhooks::list_deliveries),
        )
        .route(
            "/api/v1/webhooks/{id}/deliveries",
            post(webhooks::trigger_delivery),
        )
        .route(
            "/api/v1/webhooks/{webhook_id}/deliveries/{delivery_id}/replay",
            post(webhooks::replay_delivery),
        )
        .route("/api/v1/openapi.json", get(crate::openapi::serve_json))
        .route("/api/v1/openapi.yaml", get(crate::openapi::serve_yaml))
        .fallback(fallback)
        .with_state(shared_state.clone())
        .layer(from_fn_with_state(
            shared_state.clone(),
            rate_limit_middleware,
        ))
        .layer(from_fn_with_state(shared_state, request_limits_middleware));

    api.layer(
        ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
            .layer(CompressionLayer::new())
            .layer(cors)
            .layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                timeout,
            ))
            .layer(DefaultBodyLimit::max(body_limit)),
    )
}

fn build_cors_layer(origins: &[String]) -> CorsLayer {
    if origins.iter().any(|o| o.as_str() == "*") {
        return CorsLayer::new()
            .allow_origin(AllowOrigin::any())
            .allow_methods(Any)
            .allow_headers(Any);
    }
    let origins: Vec<HeaderValue> = origins
        .iter()
        .filter(|o| !o.is_empty())
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    if origins.is_empty() {
        // No allowed origins configured; cross-origin requests are denied.
        CorsLayer::new()
    } else {
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

async fn fallback() -> (StatusCode, Json<crate::ProblemDetails>) {
    let problem = crate::ProblemDetails {
        code: "NOT_FOUND".to_string(),
        message: "resource not found".to_string(),
        status: 404,
        request_id: None,
        field_violations: Vec::new(),
    };
    (StatusCode::NOT_FOUND, Json(problem))
}

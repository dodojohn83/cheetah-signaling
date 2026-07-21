//! Axum router setup and middleware.

use crate::ApiState;
use crate::handlers::{
    channels, devices, events, health, media, nodes, operations, ops, tenants, webhooks,
};
use crate::rate_limit::rate_limit_middleware;
use axum::{
    Router,
    body::Body,
    extract::DefaultBodyLimit,
    http::{HeaderValue, Request, Response, StatusCode, header},
    middleware::{Next, from_fn, from_fn_with_state},
    response::Json,
    routing::{delete, get, patch, post},
};
use cheetah_signal_types::{validate_traceparent, validate_tracestate};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, Any, CorsLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::Span;

/// Extension carrying the request identifier for correlation.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// Builds the public API router.
pub fn build_router(state: ApiState) -> Router {
    let timeout = Duration::from_millis(state.config.read_timeout_ms);
    let body_limit = state.config.request_body_limit_bytes;
    let cors = build_cors_layer(&state.config.cors_allowed_origins);
    let metrics = state.metrics.clone();
    let shared_state = Arc::new(state);
    let metrics_request = metrics.clone();
    let metrics_response = metrics;

    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|req: &Request<_>| {
            let traceparent = req
                .headers()
                .get("traceparent")
                .and_then(|v| v.to_str().ok())
                .and_then(validate_traceparent);
            let tracestate = req
                .headers()
                .get("tracestate")
                .and_then(|v| v.to_str().ok())
                .and_then(validate_tracestate);
            let span = tracing::info_span!(
                "http_request",
                "http.method" = tracing::field::Empty,
                "http.uri" = tracing::field::Empty,
                protocol = "http",
                traceparent = tracing::field::Empty,
                tracestate = tracing::field::Empty,
                tenant_id = tracing::field::Empty,
                request_id = tracing::field::Empty,
                node_id = tracing::field::Empty,
            );
            if let Some(tp) = traceparent {
                span.record("traceparent", tp);
            }
            if let Some(ts) = tracestate {
                span.record("tracestate", ts);
            }
            span
        })
        .on_request(move |req: &Request<_>, span: &Span| {
            span.record("http.method", tracing::field::display(req.method()));
            span.record("http.uri", req.uri().to_string());
            metrics_request.record_request();
        })
        .on_response(
            move |response: &Response<_>, latency: Duration, span: &Span| {
                let status = response.status();
                metrics_response.record_response(status);
                metrics_response.record_duration(latency);
                tracing::info!(
                    parent: span,
                    status = status.as_u16(),
                    "finished processing request"
                );
            },
        );

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
        .route(
            "/api/v1/admin/media-nodes/{node_id}/drain",
            post(nodes::drain_media_node),
        )
        .route(
            "/api/v1/admin/media-nodes/{node_id}/force-cleanup",
            post(nodes::force_cleanup_media_node),
        )
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
        .route("/api/v1/admin/validate-config", post(ops::validate_config))
        .route("/api/v1/admin/db-status", get(ops::db_status))
        .route("/api/v1/admin/db-migrate", post(ops::db_migrate))
        .route("/api/v1/admin/node-drain", post(ops::node_drain))
        .route(
            "/api/v1/admin/devices/{id}/diagnostics",
            get(ops::device_diagnostics),
        )
        .route("/api/v1/admin/outbox-replay", post(ops::outbox_replay))
        .route("/api/v1/admin/reconcile", post(ops::reconcile))
        .fallback(fallback)
        .with_state(shared_state.clone())
        .layer(from_fn_with_state(shared_state, rate_limit_middleware));

    api.layer(
        ServiceBuilder::new()
            .layer(from_fn(request_id_and_trace_middleware))
            .layer(trace_layer)
            .layer(CompressionLayer::new())
            .layer(cors)
            .layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                timeout,
            ))
            .layer(DefaultBodyLimit::max(body_limit)),
    )
}

/// Propagates W3C trace headers, assigns a stable `x-request-id`, and injects
/// `request_id` into RFC 9457 Problem Details error bodies when missing.
async fn request_id_and_trace_middleware(mut request: Request<Body>, next: Next) -> Response<Body> {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

    let traceparent = request
        .headers()
        .get("traceparent")
        .cloned()
        .filter(|v| v.to_str().ok().and_then(validate_traceparent).is_some());
    let tracestate = request
        .headers()
        .get("tracestate")
        .cloned()
        .filter(|v| v.to_str().ok().and_then(validate_tracestate).is_some());

    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    let mut response = next.run(request).await;

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    if let Some(tp) = traceparent {
        response.headers_mut().insert("traceparent", tp);
    }
    if let Some(ts) = tracestate {
        response.headers_mut().insert("tracestate", ts);
    }

    inject_request_id_into_problem_details(response, &request_id).await
}

async fn inject_request_id_into_problem_details(
    response: Response<Body>,
    request_id: &str,
) -> Response<Body> {
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return response;
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.contains("json") {
        return response;
    }

    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, 64 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return Response::from_parts(parts, Body::empty());
        }
    };

    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let Some(obj) = value.as_object_mut() else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    // Only rewrite RFC 9457-style problem objects that expose a stable code.
    if !obj.contains_key("code") {
        return Response::from_parts(parts, Body::from(bytes));
    }
    let missing = match obj.get("request_id") {
        None => true,
        Some(v) => v.is_null() || v.as_str().is_some_and(str::is_empty),
    };
    if !missing {
        return Response::from_parts(parts, Body::from(bytes));
    }
    obj.insert(
        "request_id".to_string(),
        serde_json::Value::String(request_id.to_string()),
    );
    match serde_json::to_vec(&value) {
        Ok(new_bytes) => Response::from_parts(parts, Body::from(new_bytes)),
        Err(_) => Response::from_parts(parts, Body::from(bytes)),
    }
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

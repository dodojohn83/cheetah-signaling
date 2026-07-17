//! Request size and shape limits enforced before authentication.

use crate::{ApiState, HttpError};
use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

/// Axum middleware that rejects oversized or malformed requests before they
/// reach authentication or handlers.
///
/// Checks:
/// - URI length (path + query) does not exceed `max_request_uri_length`.
/// - Header count does not exceed `max_request_headers`.
/// - The `Content-Length` header, if present, is within `request_body_limit_bytes`.
///
/// These limits run before the rate-limiter and authentication extractors so
/// that cheap rejection happens as early as possible.
pub async fn request_limits_middleware(
    State(state): State<Arc<ApiState>>,
    req: Request,
    next: Next,
) -> Response {
    if let Some(err) = check_limits(&req, &state.config) {
        return err.into_response();
    }
    next.run(req).await
}

fn check_limits(req: &Request<Body>, config: &crate::ApiConfig) -> Option<HttpError> {
    let uri = req.uri().to_string();
    if uri.len() > config.max_request_uri_length {
        return Some(HttpError::PayloadTooLarge(format!(
            "request URI exceeds {} bytes",
            config.max_request_uri_length
        )));
    }

    if req.headers().len() > config.max_request_headers {
        return Some(HttpError::PayloadTooLarge(format!(
            "request has more than {} headers",
            config.max_request_headers
        )));
    }

    if let Some(content_length) = req.headers().get("content-length") {
        if let Ok(text) = content_length.to_str() {
            if let Ok(length) = text.parse::<usize>() {
                if length > config.request_body_limit_bytes {
                    return Some(HttpError::PayloadTooLarge(format!(
                        "content-length {} exceeds {} bytes",
                        length, config.request_body_limit_bytes
                    )));
                }
            } else {
                return Some(HttpError::PayloadTooLarge(
                    "invalid content-length header".to_string(),
                ));
            }
        } else {
            return Some(HttpError::PayloadTooLarge(
                "invalid content-length header".to_string(),
            ));
        }
    }

    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue, Request, StatusCode};

    fn test_config() -> crate::ApiConfig {
        crate::ApiConfig {
            listen_addr: "127.0.0.1".to_string(),
            port: 0,
            read_timeout_ms: 5000,
            request_body_limit_bytes: 1024,
            max_request_headers: 4,
            max_request_uri_length: 64,
            cors_allowed_origins: Vec::new(),
            rate_limit_requests_per_second: 0,
            rate_limit_burst: 0,
            webhook_delivery_interval_ms: 0,
            node_id: Default::default(),
            tls_cert_ref: None,
            tls_key_ref: None,
            security: Default::default(),
        }
    }

    fn request_with_uri(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[test]
    fn uri_within_limit_is_allowed() {
        let req = request_with_uri("/api/v1/devices");
        assert!(check_limits(&req, &test_config()).is_none());
    }

    #[test]
    fn uri_exceeding_length_is_rejected() {
        let long = format!("/api/v1/devices?{}={}", "x".repeat(50), "y".repeat(50));
        let req = request_with_uri(&long);
        let err = check_limits(&req, &test_config()).unwrap();
        assert_eq!(err.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn too_many_headers_is_rejected() {
        let mut req = request_with_uri("/api/v1/devices");
        for i in 0..5 {
            let name = HeaderName::from_bytes(format!("x-header-{i}").as_bytes()).unwrap();
            req.headers_mut()
                .insert(name, HeaderValue::from_static("value"));
        }
        let err = check_limits(&req, &test_config()).unwrap();
        assert_eq!(err.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn oversized_content_length_is_rejected() {
        let req = Request::builder()
            .uri("/api/v1/devices")
            .header("content-length", "2048")
            .body(Body::empty())
            .unwrap();
        let err = check_limits(&req, &test_config()).unwrap();
        assert_eq!(err.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}

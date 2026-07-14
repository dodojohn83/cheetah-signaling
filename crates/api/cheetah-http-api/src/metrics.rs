//! Minimal metrics exposition.

use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Shared HTTP request counters.
#[derive(Debug, Default)]
pub struct RequestMetrics {
    /// Total number of HTTP requests received.
    pub requests_total: AtomicU64,
    /// Number of failed HTTP responses.
    pub responses_failed: AtomicU64,
}

impl RequestMetrics {
    /// Increments the total request counter.
    pub fn record_request(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the failed response counter.
    pub fn record_failure(&self) {
        self.responses_failed.fetch_add(1, Ordering::Relaxed);
    }
}

/// Returns a Prometheus-compatible metrics response.
pub fn metrics_response(metrics: Arc<RequestMetrics>) -> Response {
    let body = format!(
        "# TYPE cheetah_http_requests_total counter\ncheetah_http_requests_total {}\n# TYPE cheetah_http_responses_failed_total counter\ncheetah_http_responses_failed_total {}\n",
        metrics.requests_total.load(Ordering::Relaxed),
        metrics.responses_failed.load(Ordering::Relaxed),
    );
    ([("content-type", "text/plain; version=0.0.4")], body).into_response()
}

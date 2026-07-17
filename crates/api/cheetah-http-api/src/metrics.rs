//! Minimal metrics exposition.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Shared HTTP request counters.
///
/// Metrics intentionally avoid high-cardinality labels such as tenant or
/// request IDs; those belong in structured logs and traces.
#[derive(Debug, Default)]
pub struct RequestMetrics {
    /// Total number of HTTP requests received.
    pub requests_total: AtomicU64,
    /// Number of failed HTTP responses (5xx family).
    pub responses_failed: AtomicU64,
    /// Number of successful HTTP responses (2xx family).
    pub responses_2xx: AtomicU64,
    /// Number of client error HTTP responses (4xx family).
    pub responses_4xx: AtomicU64,
    /// Number of server error HTTP responses (5xx family).
    pub responses_5xx: AtomicU64,
}

impl RequestMetrics {
    /// Increments the total request counter.
    pub fn record_request(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a completed response by status family.
    pub fn record_response(&self, status: StatusCode) {
        if status.is_server_error() {
            self.responses_5xx.fetch_add(1, Ordering::Relaxed);
            self.responses_failed.fetch_add(1, Ordering::Relaxed);
        } else if status.is_client_error() {
            self.responses_4xx.fetch_add(1, Ordering::Relaxed);
        } else if status.is_success() {
            self.responses_2xx.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Returns a Prometheus-compatible metrics response.
pub fn metrics_response(metrics: Arc<RequestMetrics>) -> Response {
    let body = format!(
        "# TYPE cheetah_http_requests_total counter\n\
         cheetah_http_requests_total {}\n\
         # TYPE cheetah_http_responses_failed_total counter\n\
         cheetah_http_responses_failed_total {}\n\
         # TYPE cheetah_http_responses_2xx_total counter\n\
         cheetah_http_responses_2xx_total {}\n\
         # TYPE cheetah_http_responses_4xx_total counter\n\
         cheetah_http_responses_4xx_total {}\n\
         # TYPE cheetah_http_responses_5xx_total counter\n\
         cheetah_http_responses_5xx_total {}\n",
        metrics.requests_total.load(Ordering::Relaxed),
        metrics.responses_failed.load(Ordering::Relaxed),
        metrics.responses_2xx.load(Ordering::Relaxed),
        metrics.responses_4xx.load(Ordering::Relaxed),
        metrics.responses_5xx.load(Ordering::Relaxed),
    );
    ([("content-type", "text/plain; version=0.0.4")], body).into_response()
}

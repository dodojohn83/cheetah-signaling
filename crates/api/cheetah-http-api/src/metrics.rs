//! Minimal metrics exposition.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use cheetah_signal_types::MetricsExporter;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Upper bounds in seconds for the response-duration histogram.
/// A final `+Inf` bucket is appended in [`RequestMetrics::default`].
const DURATION_BUCKET_BOUNDS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Shared HTTP request counters and response-duration histogram.
///
/// Metrics intentionally avoid high-cardinality labels such as tenant or
/// request IDs; those belong in structured logs and traces.
#[derive(Debug)]
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
    /// Number of GB28181 events dropped because the application sink queue was full.
    pub gb28181_events_dropped_total: AtomicU64,
    /// Number of GB28181 events admitted into the application sink queue.
    pub gb28181_events_admitted_total: AtomicU64,
    /// Number of redundant GB28181 events coalesced before reaching the sink queue.
    pub gb28181_events_coalesced_total: AtomicU64,
    /// Number of low-priority GB28181 events shed under overload.
    pub gb28181_events_shed_total: AtomicU64,
    /// Number of GB28181 events placed in the dead-letter queue for redrive.
    pub gb28181_events_dead_lettered_total: AtomicU64,
    /// Number of GB28181 events successfully redriven from the dead-letter queue.
    pub gb28181_events_redriven_total: AtomicU64,
    /// Number of GB28181 events dropped after exhausting dead-letter redrive budget.
    pub gb28181_events_redrive_exhausted_total: AtomicU64,
    /// Sum of response durations in nanoseconds.
    response_duration_sum_ns: AtomicU64,
    /// Cumulative response-duration histogram buckets, ending with `+Inf`.
    response_duration_buckets: Vec<AtomicU64>,
}

impl Default for RequestMetrics {
    fn default() -> Self {
        let bucket_count = DURATION_BUCKET_BOUNDS.len() + 1;
        Self {
            requests_total: AtomicU64::new(0),
            responses_failed: AtomicU64::new(0),
            responses_2xx: AtomicU64::new(0),
            responses_4xx: AtomicU64::new(0),
            responses_5xx: AtomicU64::new(0),
            gb28181_events_dropped_total: AtomicU64::new(0),
            gb28181_events_admitted_total: AtomicU64::new(0),
            gb28181_events_coalesced_total: AtomicU64::new(0),
            gb28181_events_shed_total: AtomicU64::new(0),
            gb28181_events_dead_lettered_total: AtomicU64::new(0),
            gb28181_events_redriven_total: AtomicU64::new(0),
            gb28181_events_redrive_exhausted_total: AtomicU64::new(0),
            response_duration_sum_ns: AtomicU64::new(0),
            response_duration_buckets: (0..bucket_count).map(|_| AtomicU64::new(0)).collect(),
        }
    }
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

    /// Records a GB28181 event dropped due to a full sink queue.
    pub fn record_gb28181_event_dropped(&self) {
        self.gb28181_events_dropped_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a GB28181 event admitted into the sink queue.
    pub fn record_gb28181_event_admitted(&self) {
        self.gb28181_events_admitted_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a redundant GB28181 event that was coalesced away.
    pub fn record_gb28181_event_coalesced(&self) {
        self.gb28181_events_coalesced_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a low-priority GB28181 event shed under overload.
    pub fn record_gb28181_event_shed(&self) {
        self.gb28181_events_shed_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a GB28181 event placed in the dead-letter queue.
    pub fn record_gb28181_event_dead_lettered(&self) {
        self.gb28181_events_dead_lettered_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a GB28181 event successfully redriven from the dead-letter queue.
    pub fn record_gb28181_event_redriven(&self) {
        self.gb28181_events_redriven_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a GB28181 event dropped after exhausting dead-letter redrive budget.
    pub fn record_gb28181_event_redrive_exhausted(&self) {
        self.gb28181_events_redrive_exhausted_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records the duration of a completed response for the histogram.
    pub fn record_duration(&self, duration: Duration) {
        let ns = duration.as_nanos() as u64;
        self.response_duration_sum_ns
            .fetch_add(ns, Ordering::Relaxed);

        let seconds = duration.as_secs_f64();
        for (i, bound) in DURATION_BUCKET_BOUNDS.iter().enumerate() {
            if seconds <= *bound {
                self.response_duration_buckets[i].fetch_add(1, Ordering::Relaxed);
                // Histogram buckets are cumulative, so also increment every
                // larger bucket.
                for bucket in self.response_duration_buckets.iter().skip(i + 1) {
                    bucket.fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
        }
        // Larger than all finite bounds: place in `+Inf` bucket.
        if let Some(inf) = self.response_duration_buckets.last() {
            inf.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Returns a Prometheus-compatible metrics response.
pub fn metrics_response(
    metrics: Arc<RequestMetrics>,
    media_metrics: Option<Arc<dyn MetricsExporter>>,
    gb_metrics: Option<Arc<dyn MetricsExporter>>,
) -> Response {
    let requests_total = metrics.requests_total.load(Ordering::Relaxed);
    let responses_failed = metrics.responses_failed.load(Ordering::Relaxed);
    let responses_2xx = metrics.responses_2xx.load(Ordering::Relaxed);
    let responses_4xx = metrics.responses_4xx.load(Ordering::Relaxed);
    let responses_5xx = metrics.responses_5xx.load(Ordering::Relaxed);
    let gb28181_events_dropped_total = metrics.gb28181_events_dropped_total.load(Ordering::Relaxed);
    let gb28181_events_admitted_total = metrics
        .gb28181_events_admitted_total
        .load(Ordering::Relaxed);
    let gb28181_events_coalesced_total = metrics
        .gb28181_events_coalesced_total
        .load(Ordering::Relaxed);
    let gb28181_events_shed_total = metrics.gb28181_events_shed_total.load(Ordering::Relaxed);
    let gb28181_events_dead_lettered_total = metrics
        .gb28181_events_dead_lettered_total
        .load(Ordering::Relaxed);
    let gb28181_events_redriven_total = metrics
        .gb28181_events_redriven_total
        .load(Ordering::Relaxed);
    let gb28181_events_redrive_exhausted_total = metrics
        .gb28181_events_redrive_exhausted_total
        .load(Ordering::Relaxed);

    let sum_ns = metrics.response_duration_sum_ns.load(Ordering::Relaxed);
    let sum_seconds = (sum_ns as f64) / 1_000_000_000.0;

    let mut body = format!(
        "# TYPE cheetah_http_requests_total counter\n\
         cheetah_http_requests_total {requests_total}\n\
         # TYPE cheetah_http_responses_failed_total counter\n\
         cheetah_http_responses_failed_total {responses_failed}\n\
         # TYPE cheetah_http_responses_2xx_total counter\n\
         cheetah_http_responses_2xx_total {responses_2xx}\n\
         # TYPE cheetah_http_responses_4xx_total counter\n\
         cheetah_http_responses_4xx_total {responses_4xx}\n\
         # TYPE cheetah_http_responses_5xx_total counter\n\
         cheetah_http_responses_5xx_total {responses_5xx}\n\
         # TYPE cheetah_gb28181_events_dropped_total counter\n\
         cheetah_gb28181_events_dropped_total {gb28181_events_dropped_total}\n\
         # TYPE cheetah_gb28181_events_admitted_total counter\n\
         cheetah_gb28181_events_admitted_total {gb28181_events_admitted_total}\n\
         # TYPE cheetah_gb28181_events_coalesced_total counter\n\
         cheetah_gb28181_events_coalesced_total {gb28181_events_coalesced_total}\n\
         # TYPE cheetah_gb28181_events_shed_total counter\n\
         cheetah_gb28181_events_shed_total {gb28181_events_shed_total}\n\
         # TYPE cheetah_gb28181_events_dead_lettered_total counter\n\
         cheetah_gb28181_events_dead_lettered_total {gb28181_events_dead_lettered_total}\n\
         # TYPE cheetah_gb28181_events_redriven_total counter\n\
         cheetah_gb28181_events_redriven_total {gb28181_events_redriven_total}\n\
         # TYPE cheetah_gb28181_events_redrive_exhausted_total counter\n\
         cheetah_gb28181_events_redrive_exhausted_total {gb28181_events_redrive_exhausted_total}\n\
         # TYPE cheetah_http_response_duration_seconds histogram\n"
    );

    for (i, bound) in DURATION_BUCKET_BOUNDS.iter().enumerate() {
        let count = metrics.response_duration_buckets[i].load(Ordering::Relaxed);
        body.push_str(&format!(
            "cheetah_http_response_duration_seconds_bucket{{le=\"{bound}\"}} {count}\n"
        ));
    }
    if let Some(inf_bucket) = metrics.response_duration_buckets.last() {
        let inf_count = inf_bucket.load(Ordering::Relaxed);
        body.push_str(&format!(
            "cheetah_http_response_duration_seconds_bucket{{le=\"+Inf\"}} {inf_count}\n"
        ));
    }
    let observation_count = metrics
        .response_duration_buckets
        .last()
        .map(|b| b.load(Ordering::Relaxed))
        .unwrap_or(0);
    body.push_str(&format!(
        "cheetah_http_response_duration_seconds_sum {sum_seconds}\n\
         cheetah_http_response_duration_seconds_count {observation_count}\n"
    ));

    if let Some(mm) = media_metrics {
        body.push_str(&mm.prometheus_text());
    }
    if let Some(gm) = gb_metrics {
        body.push_str(&gm.prometheus_text());
    }

    ([("content-type", "text/plain; version=0.0.4")], body).into_response()
}

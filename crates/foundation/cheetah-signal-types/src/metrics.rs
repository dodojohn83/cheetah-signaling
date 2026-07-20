//! Minimal metrics exporter abstraction.

/// A metric source that can render itself as Prometheus text.
pub trait MetricsExporter: Send + Sync {
    /// Returns Prometheus exposition format text for the current snapshot.
    fn prometheus_text(&self) -> String;
}

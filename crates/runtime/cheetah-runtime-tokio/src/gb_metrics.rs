//! GB28181 runtime/application metrics aggregator.
//!
//! [`GbMetrics`] is the single sink for GB28181 observability. It aggregates
//! runtime-derived gauges (per-shard mailbox depth, active actors, timer lag)
//! fed from the runtime, and application gauges/counters (device presence,
//! commands, catalog fragments, media sessions, cascade links) fed through the
//! [`GbMetricsRecorder`] port, then renders them as Prometheus text.
//!
//! ## Bounded cardinality
//!
//! Label cardinality is fixed at construction. Per-shard series are bounded by
//! the configured shard count; every other series is keyed only by the closed
//! enums in [`cheetah_signal_types::gb_metrics`]. Tenant, device, channel and
//! session identifiers are never used as labels.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use cheetah_runtime_api::RuntimeMetricsSnapshot;
use cheetah_signal_types::gb_metrics::{
    GbCommandMethod, GbCommandOutcome, GbDevicePresence, GbMediaSessionState, GbMetricsRecorder,
};
use cheetah_signal_types::metrics::MetricsExporter;

use crate::health::{HealthThresholds, RuntimeHealth, RuntimeHealthSource};

/// Aggregates GB28181 runtime and application metrics with bounded labels.
#[derive(Debug)]
pub struct GbMetrics {
    shard_mailbox_depth: Vec<AtomicU64>,
    active_actors: AtomicU64,
    timer_lag_ms: AtomicU64,
    active_operations: AtomicU64,
    device_total: Vec<AtomicU64>,
    command_total: Vec<AtomicU64>,
    catalog_fragment_total: AtomicU64,
    media_session_total: Vec<AtomicU64>,
    cascade_link_total: AtomicU64,
    thresholds: HealthThresholds,
}

fn zeroed(len: usize) -> Vec<AtomicU64> {
    (0..len).map(|_| AtomicU64::new(0)).collect()
}

impl GbMetrics {
    /// Creates a metrics aggregator for `shard_count` shards using default
    /// health thresholds derived from the shard mailbox capacity.
    pub fn new(shard_count: usize, shard_mailbox_capacity: u64) -> Self {
        Self::with_thresholds(
            shard_count,
            HealthThresholds::from_mailbox_capacity(shard_mailbox_capacity),
        )
    }

    /// Creates a metrics aggregator with explicit health thresholds.
    pub fn with_thresholds(shard_count: usize, thresholds: HealthThresholds) -> Self {
        let shard_count = shard_count.max(1);
        Self {
            shard_mailbox_depth: zeroed(shard_count),
            active_actors: AtomicU64::new(0),
            timer_lag_ms: AtomicU64::new(0),
            active_operations: AtomicU64::new(0),
            device_total: zeroed(GbDevicePresence::ALL.len()),
            command_total: zeroed(GbCommandMethod::ALL.len() * GbCommandOutcome::ALL.len()),
            catalog_fragment_total: AtomicU64::new(0),
            media_session_total: zeroed(GbMediaSessionState::ALL.len()),
            cascade_link_total: AtomicU64::new(0),
            thresholds,
        }
    }

    /// Number of shards this aggregator reports depth for.
    pub fn shard_count(&self) -> usize {
        self.shard_mailbox_depth.len()
    }

    /// Feeds runtime-derived gauges from a consistent runtime snapshot and the
    /// per-shard mailbox depths. Extra shard depths beyond the configured shard
    /// count are ignored so the series set stays bounded.
    pub fn record_runtime_sample(
        &self,
        snapshot: &RuntimeMetricsSnapshot,
        shard_mailbox_depths: &[u64],
    ) {
        self.active_actors
            .store(snapshot.active_actors, Ordering::Relaxed);
        self.timer_lag_ms
            .store(snapshot.timer_lag_ms, Ordering::Relaxed);
        for (slot, depth) in self.shard_mailbox_depth.iter().zip(shard_mailbox_depths) {
            slot.store(*depth, Ordering::Relaxed);
        }
    }

    fn command_slot(&self, method: GbCommandMethod, outcome: GbCommandOutcome) -> &AtomicU64 {
        let index = method.index() * GbCommandOutcome::ALL.len() + outcome.index();
        &self.command_total[index]
    }

    fn max_shard_mailbox_depth(&self) -> u64 {
        self.shard_mailbox_depth
            .iter()
            .map(|slot| slot.load(Ordering::Relaxed))
            .max()
            .unwrap_or(0)
    }
}

impl GbMetricsRecorder for GbMetrics {
    fn record_command(&self, method: GbCommandMethod, outcome: GbCommandOutcome) {
        self.command_slot(method, outcome)
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_catalog_fragment(&self) {
        self.catalog_fragment_total.fetch_add(1, Ordering::Relaxed);
    }

    fn set_active_operations(&self, count: u64) {
        self.active_operations.store(count, Ordering::Relaxed);
    }

    fn set_device_gauge(&self, presence: GbDevicePresence, count: u64) {
        self.device_total[presence.index()].store(count, Ordering::Relaxed);
    }

    fn set_media_session_gauge(&self, state: GbMediaSessionState, count: u64) {
        self.media_session_total[state.index()].store(count, Ordering::Relaxed);
    }

    fn set_cascade_link_total(&self, count: u64) {
        self.cascade_link_total.store(count, Ordering::Relaxed);
    }
}

impl RuntimeHealthSource for GbMetrics {
    fn runtime_health(&self) -> RuntimeHealth {
        RuntimeHealth::evaluate(
            &self.thresholds,
            self.max_shard_mailbox_depth(),
            self.active_actors.load(Ordering::Relaxed),
            self.timer_lag_ms.load(Ordering::Relaxed),
        )
    }
}

impl MetricsExporter for GbMetrics {
    fn prometheus_text(&self) -> String {
        let mut out = String::new();

        out.push_str(
            "# HELP gb28181_shard_mailbox_depth Current occupancy of each GB28181 shard mailbox.\n",
        );
        out.push_str("# TYPE gb28181_shard_mailbox_depth gauge\n");
        for (shard, slot) in self.shard_mailbox_depth.iter().enumerate() {
            let _ = writeln!(
                out,
                "gb28181_shard_mailbox_depth{{shard=\"{shard}\"}} {}",
                slot.load(Ordering::Relaxed)
            );
        }

        out.push_str("# HELP gb28181_active_actors Currently loaded GB28181 device actors.\n");
        out.push_str("# TYPE gb28181_active_actors gauge\n");
        let _ = writeln!(
            out,
            "gb28181_active_actors {}",
            self.active_actors.load(Ordering::Relaxed)
        );

        out.push_str(
            "# HELP gb28181_timer_lag_seconds Most recent timer-wheel tick lag in seconds.\n",
        );
        out.push_str("# TYPE gb28181_timer_lag_seconds gauge\n");
        let lag_seconds = self.timer_lag_ms.load(Ordering::Relaxed) as f64 / 1000.0;
        let _ = writeln!(out, "gb28181_timer_lag_seconds {lag_seconds}");

        out.push_str(
            "# HELP gb28181_active_operations In-flight GB28181 application operations.\n",
        );
        out.push_str("# TYPE gb28181_active_operations gauge\n");
        let _ = writeln!(
            out,
            "gb28181_active_operations {}",
            self.active_operations.load(Ordering::Relaxed)
        );

        out.push_str("# HELP gb28181_device_total Known GB28181 devices by presence.\n");
        out.push_str("# TYPE gb28181_device_total gauge\n");
        for presence in GbDevicePresence::ALL {
            let _ = writeln!(
                out,
                "gb28181_device_total{{presence=\"{}\"}} {}",
                presence.as_str(),
                self.device_total[presence.index()].load(Ordering::Relaxed)
            );
        }

        out.push_str(
            "# HELP gb28181_command_total GB28181 commands dispatched by method and outcome.\n",
        );
        out.push_str("# TYPE gb28181_command_total counter\n");
        for method in GbCommandMethod::ALL {
            for outcome in GbCommandOutcome::ALL {
                let _ = writeln!(
                    out,
                    "gb28181_command_total{{method=\"{}\",outcome=\"{}\"}} {}",
                    method.as_str(),
                    outcome.as_str(),
                    self.command_slot(method, outcome).load(Ordering::Relaxed)
                );
            }
        }

        out.push_str("# HELP gb28181_catalog_fragment_total Received GB28181 catalog fragments.\n");
        out.push_str("# TYPE gb28181_catalog_fragment_total counter\n");
        let _ = writeln!(
            out,
            "gb28181_catalog_fragment_total {}",
            self.catalog_fragment_total.load(Ordering::Relaxed)
        );

        out.push_str("# HELP gb28181_media_session_total GB28181 media sessions by state.\n");
        out.push_str("# TYPE gb28181_media_session_total gauge\n");
        for state in GbMediaSessionState::ALL {
            let _ = writeln!(
                out,
                "gb28181_media_session_total{{state=\"{}\"}} {}",
                state.as_str(),
                self.media_session_total[state.index()].load(Ordering::Relaxed)
            );
        }

        out.push_str("# HELP gb28181_cascade_link_total Established GB28181 cascade links.\n");
        out.push_str("# TYPE gb28181_cascade_link_total gauge\n");
        let _ = writeln!(
            out,
            "gb28181_cascade_link_total {}",
            self.cascade_link_total.load(Ordering::Relaxed)
        );

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_all_nine_metric_families() {
        let metrics = GbMetrics::new(4, 8192);
        let text = metrics.prometheus_text();
        for name in [
            "gb28181_shard_mailbox_depth",
            "gb28181_active_actors",
            "gb28181_timer_lag_seconds",
            "gb28181_active_operations",
            "gb28181_device_total",
            "gb28181_command_total",
            "gb28181_catalog_fragment_total",
            "gb28181_media_session_total",
            "gb28181_cascade_link_total",
        ] {
            assert!(text.contains(name), "missing metric {name}");
        }
    }

    #[test]
    fn shard_series_are_bounded_by_shard_count() {
        let metrics = GbMetrics::new(3, 100);
        // Feeding extra shard depths must not create extra series.
        metrics.record_runtime_sample(
            &RuntimeMetricsSnapshot {
                active_actors: 7,
                timer_lag_ms: 250,
                ..Default::default()
            },
            &[1, 2, 3, 4, 5, 6],
        );
        let text = metrics.prometheus_text();
        let shard_lines = text
            .lines()
            .filter(|l| l.starts_with("gb28181_shard_mailbox_depth{"))
            .count();
        assert_eq!(shard_lines, 3);
        assert!(text.contains("gb28181_active_actors 7"));
        assert!(text.contains("gb28181_timer_lag_seconds 0.25"));
    }

    #[test]
    fn command_cardinality_is_fixed_regardless_of_volume() {
        let metrics = GbMetrics::new(1, 100);
        for _ in 0..1000 {
            metrics.record_command(GbCommandMethod::Ptz, GbCommandOutcome::Succeeded);
        }
        let text = metrics.prometheus_text();
        let command_lines = text
            .lines()
            .filter(|l| l.starts_with("gb28181_command_total{"))
            .count();
        assert_eq!(
            command_lines,
            GbCommandMethod::ALL.len() * GbCommandOutcome::ALL.len()
        );
        assert!(text.contains("gb28181_command_total{method=\"ptz\",outcome=\"succeeded\"} 1000"));
    }

    #[test]
    fn no_identifiers_leak_into_labels() {
        let metrics = GbMetrics::new(2, 100);
        metrics.set_device_gauge(GbDevicePresence::Online, 42);
        metrics.record_command(GbCommandMethod::Query, GbCommandOutcome::Failed);
        let text = metrics.prometheus_text();
        // Only bounded label keys may appear.
        for line in text.lines() {
            let (Some(open), Some(close)) = (line.find('{'), line.find('}')) else {
                continue;
            };
            let labels = &line[open + 1..close];
            for pair in labels.split(',') {
                let key = pair.split('=').next().unwrap_or(pair);
                assert!(
                    matches!(key, "shard" | "presence" | "method" | "outcome" | "state"),
                    "unexpected label key {key}"
                );
            }
        }
    }

    #[test]
    fn health_reflects_recorded_pressure() {
        let metrics = GbMetrics::new(2, 100);
        metrics.record_runtime_sample(
            &RuntimeMetricsSnapshot {
                active_actors: 3,
                timer_lag_ms: 0,
                ..Default::default()
            },
            &[100, 10],
        );
        let health = metrics.runtime_health();
        assert!(!health.ready);
        assert_eq!(health.max_shard_mailbox_depth, 100);
    }
}

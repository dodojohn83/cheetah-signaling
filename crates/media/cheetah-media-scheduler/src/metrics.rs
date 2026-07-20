//! Low-cardinality media scheduler metrics.

use cheetah_signal_types::{Clock, MetricsExporter};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const RPC_DURATION_BUCKETS_S: [f64; 9] = [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0];

/// Metrics for the media scheduler, registry, event consumer and reconciler.
///
/// Counters are intentionally low cardinality. Node IDs and tenant IDs are never
/// used as labels; per-node load is aggregated into a distribution instead.
#[derive(Debug, Default)]
pub struct MediaMetrics {
    media_nodes_active: AtomicU64,
    media_nodes_expired: AtomicU64,
    media_nodes_draining: AtomicU64,
    reservations_total: AtomicU64,
    reservations_success: AtomicU64,
    reservations_rejected: AtomicU64,
    rpc_total: AtomicU64,
    rpc_errors: AtomicU64,
    rpc_duration_sum_ns: AtomicU64,
    rpc_duration_buckets: Vec<AtomicU64>,
    event_lag_ms: AtomicU64,
    event_gaps: AtomicU64,
    event_reconnects: AtomicU64,
    reconcile_scanned: AtomicU64,
    reconcile_repaired: AtomicU64,
    reconcile_failed: AtomicU64,
    reconcile_orphans: AtomicU64,
    node_load_sum: AtomicU64,
    node_load_count: AtomicU64,
    register_total: AtomicU64,
    drain_total: AtomicU64,
    deregister_total: AtomicU64,
    forced_cleanup_total: AtomicU64,
    reservation_rejected_reasons: Mutex<HashMap<String, u64>>,
}

impl MediaMetrics {
    /// Creates a new metrics instance.
    pub fn new() -> Self {
        Self {
            rpc_duration_buckets: (0..RPC_DURATION_BUCKETS_S.len() + 1)
                .map(|_| AtomicU64::new(0))
                .collect(),
            ..Self::default()
        }
    }

    /// Records a node snapshot, updating active/expired/draining counts.
    pub fn record_node_snapshot(&self, nodes: &[cheetah_domain::MediaNode], clock: &dyn Clock) {
        let now = clock.now_wall();
        let mut active = 0;
        let mut expired = 0;
        let mut draining = 0;
        let mut load_sum: u64 = 0;
        let mut load_count: u64 = 0;

        for node in nodes {
            let lease_expired = node.lease_until.map(|until| until <= now).unwrap_or(false);
            if lease_expired {
                expired += 1;
            } else {
                active += 1;
            }
            if node.draining {
                draining += 1;
            }
            if node.status != cheetah_domain::NodeStatus::Left {
                load_sum = load_sum.saturating_add(node.load);
                load_count = load_count.saturating_add(1);
            }
        }

        self.media_nodes_active.store(active, Ordering::Relaxed);
        self.media_nodes_expired.store(expired, Ordering::Relaxed);
        self.media_nodes_draining.store(draining, Ordering::Relaxed);
        self.node_load_sum.store(load_sum, Ordering::Relaxed);
        self.node_load_count.store(load_count, Ordering::Relaxed);
    }

    /// Records a media node registration.
    pub fn record_register(&self) {
        self.register_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a media node drain request.
    pub fn record_drain(&self) {
        self.drain_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a media node deregistration.
    pub fn record_deregister(&self) {
        self.deregister_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a forced cleanup of an orphan media session.
    pub fn record_forced_cleanup(&self) {
        self.forced_cleanup_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a reservation attempt. `reason` is a low-cardinality label such
    /// as `no_node`, `capacity_exhausted` or `invalid_argument`; it is ignored on
    /// success.
    pub fn record_reservation(&self, success: bool, reason: Option<&str>) {
        self.reservations_total.fetch_add(1, Ordering::Relaxed);
        if success {
            self.reservations_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.reservations_rejected.fetch_add(1, Ordering::Relaxed);
            if let Some(reason) = reason {
                let mut map = self
                    .reservation_rejected_reasons
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                *map.entry(reason.to_string()).or_insert(0) += 1;
            }
        }
    }

    /// Records an RPC call, including its duration and whether it errored.
    pub fn record_rpc(&self, duration: Duration, error: bool) {
        self.rpc_total.fetch_add(1, Ordering::Relaxed);
        if error {
            self.rpc_errors.fetch_add(1, Ordering::Relaxed);
        }
        let ns = duration.as_nanos() as u64;
        self.rpc_duration_sum_ns.fetch_add(ns, Ordering::Relaxed);

        let seconds = duration.as_secs_f64();
        for (i, bound) in RPC_DURATION_BUCKETS_S.iter().enumerate() {
            if seconds <= *bound {
                self.rpc_duration_buckets[i].fetch_add(1, Ordering::Relaxed);
                for bucket in self.rpc_duration_buckets.iter().skip(i + 1) {
                    bucket.fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
        }
        if let Some(inf) = self.rpc_duration_buckets.last() {
            inf.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Records the observed event lag in milliseconds.
    pub fn record_event_lag_ms(&self, lag_ms: u64) {
        self.event_lag_ms.store(lag_ms, Ordering::Relaxed);
    }

    /// Records a detected event sequence gap.
    pub fn record_event_gap(&self) {
        self.event_gaps.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a subscription reconnect to a media node.
    pub fn record_event_reconnect(&self) {
        self.event_reconnects.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a reconciliation report.
    pub fn record_reconcile(&self, scanned: u64, repaired: u64, failed: u64, orphans: u64) {
        self.reconcile_scanned.fetch_add(scanned, Ordering::Relaxed);
        self.reconcile_repaired
            .fetch_add(repaired, Ordering::Relaxed);
        self.reconcile_failed.fetch_add(failed, Ordering::Relaxed);
        self.reconcile_orphans.fetch_add(orphans, Ordering::Relaxed);
    }
}

impl MetricsExporter for MediaMetrics {
    fn prometheus_text(&self) -> String {
        let active = self.media_nodes_active.load(Ordering::Relaxed);
        let expired = self.media_nodes_expired.load(Ordering::Relaxed);
        let draining = self.media_nodes_draining.load(Ordering::Relaxed);
        let reservations_total = self.reservations_total.load(Ordering::Relaxed);
        let reservations_success = self.reservations_success.load(Ordering::Relaxed);
        let reservations_rejected = self.reservations_rejected.load(Ordering::Relaxed);
        let rejected_reasons: Vec<(String, u64)> = self
            .reservation_rejected_reasons
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let rpc_total = self.rpc_total.load(Ordering::Relaxed);
        let rpc_errors = self.rpc_errors.load(Ordering::Relaxed);
        let rpc_sum_ns = self.rpc_duration_sum_ns.load(Ordering::Relaxed);
        let rpc_sum_seconds = (rpc_sum_ns as f64) / 1_000_000_000.0;
        let event_lag_ms = self.event_lag_ms.load(Ordering::Relaxed);
        let event_gaps = self.event_gaps.load(Ordering::Relaxed);
        let event_reconnects = self.event_reconnects.load(Ordering::Relaxed);
        let reconcile_scanned = self.reconcile_scanned.load(Ordering::Relaxed);
        let reconcile_repaired = self.reconcile_repaired.load(Ordering::Relaxed);
        let reconcile_failed = self.reconcile_failed.load(Ordering::Relaxed);
        let reconcile_orphans = self.reconcile_orphans.load(Ordering::Relaxed);
        let node_load_sum = self.node_load_sum.load(Ordering::Relaxed);
        let node_load_count = self.node_load_count.load(Ordering::Relaxed);
        let register_total = self.register_total.load(Ordering::Relaxed);
        let drain_total = self.drain_total.load(Ordering::Relaxed);
        let deregister_total = self.deregister_total.load(Ordering::Relaxed);
        let forced_cleanup_total = self.forced_cleanup_total.load(Ordering::Relaxed);

        let mut body = format!(
            "# TYPE cheetah_media_nodes_active gauge\n\
             cheetah_media_nodes_active {active}\n\
             # TYPE cheetah_media_nodes_expired gauge\n\
             cheetah_media_nodes_expired {expired}\n\
             # TYPE cheetah_media_nodes_draining gauge\n\
             cheetah_media_nodes_draining {draining}\n\
             # TYPE cheetah_media_reservations_total counter\n\
             cheetah_media_reservations_total {reservations_total}\n\
             # TYPE cheetah_media_reservations_success_total counter\n\
             cheetah_media_reservations_success_total {reservations_success}\n\
             # TYPE cheetah_media_reservations_rejected_total counter\n\
             cheetah_media_reservations_rejected_total {reservations_rejected}\n\n\
             # TYPE cheetah_media_rpc_total counter\n\
             cheetah_media_rpc_total {rpc_total}\n\
             # TYPE cheetah_media_rpc_errors_total counter\n\
             cheetah_media_rpc_errors_total {rpc_errors}\n\
             # TYPE cheetah_media_event_lag_ms gauge\n\
             cheetah_media_event_lag_ms {event_lag_ms}\n\
             # TYPE cheetah_media_event_gaps_total counter\n\
             cheetah_media_event_gaps_total {event_gaps}\n\
             # TYPE cheetah_media_event_reconnects_total counter\n\
             cheetah_media_event_reconnects_total {event_reconnects}\n\
             # TYPE cheetah_media_reconcile_scanned_total counter\n\
             cheetah_media_reconcile_scanned_total {reconcile_scanned}\n\
             # TYPE cheetah_media_reconcile_repaired_total counter\n\
             cheetah_media_reconcile_repaired_total {reconcile_repaired}\n\
             # TYPE cheetah_media_reconcile_failed_total counter\n\
             cheetah_media_reconcile_failed_total {reconcile_failed}\n\
             # TYPE cheetah_media_reconcile_orphans_total counter\n\
             cheetah_media_reconcile_orphans_total {reconcile_orphans}\n\
             # TYPE cheetah_media_node_load_sum counter\n\
             cheetah_media_node_load_sum {node_load_sum}\n\
             # TYPE cheetah_media_node_load_count counter\n\
             cheetah_media_node_load_count {node_load_count}\n\
             # TYPE cheetah_media_register_total counter\n\
             cheetah_media_register_total {register_total}\n\
             # TYPE cheetah_media_drain_total counter\n\
             cheetah_media_drain_total {drain_total}\n\
             # TYPE cheetah_media_deregister_total counter\n\
             cheetah_media_deregister_total {deregister_total}\n\
             # TYPE cheetah_media_forced_cleanup_total counter\n\
             cheetah_media_forced_cleanup_total {forced_cleanup_total}\n"
        );

        for (reason, count) in rejected_reasons {
            body.push_str(&format!(
                "cheetah_media_reservations_rejected_total{{reason=\"{reason}\"}} {count}\n"
            ));
        }

        body.push_str("# TYPE cheetah_media_rpc_duration_seconds histogram\n");
        for (i, bound) in RPC_DURATION_BUCKETS_S.iter().enumerate() {
            let count = self.rpc_duration_buckets[i].load(Ordering::Relaxed);
            body.push_str(&format!(
                "cheetah_media_rpc_duration_seconds_bucket{{le=\"{bound}\"}} {count}\n"
            ));
        }
        if let Some(inf_bucket) = self.rpc_duration_buckets.last() {
            let inf_count = inf_bucket.load(Ordering::Relaxed);
            body.push_str(&format!(
                "cheetah_media_rpc_duration_seconds_bucket{{le=\"+Inf\"}} {inf_count}\n"
            ));
        }
        let observation_count = self
            .rpc_duration_buckets
            .last()
            .map(|b| b.load(Ordering::Relaxed))
            .unwrap_or(0);
        body.push_str(&format!(
            "cheetah_media_rpc_duration_seconds_sum {rpc_sum_seconds}\n\
             cheetah_media_rpc_duration_seconds_count {observation_count}\n"
        ));

        body
    }
}

impl MediaMetrics {
    /// Creates a shared metrics instance wrapped in an [`Arc`].
    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

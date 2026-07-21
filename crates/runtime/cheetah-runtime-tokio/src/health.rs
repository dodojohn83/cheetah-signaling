//! Runtime readiness and degraded-state health reporting.
//!
//! The GB28181 runtime is healthy while its shard mailboxes drain and its timer
//! wheel keeps up. Sustained mailbox saturation or timer lag indicates the node
//! can no longer service work within bounds; per §6.3 of the operations spec
//! such a node reports `degraded` (still serving, but pressured) or, when
//! saturation reaches the mailbox capacity, `not_ready` so orchestration stops
//! routing new work to it.
//!
//! [`RuntimeHealth`] is a bounded summary: it never enumerates device or session
//! identifiers, only aggregate gauges and a fixed set of reason codes.

/// Thresholds that classify runtime pressure into degraded / not-ready.
#[derive(Clone, Copy, Debug)]
pub struct HealthThresholds {
    /// Configured shard mailbox capacity. A shard at this depth is fully
    /// saturated and the runtime reports not-ready.
    pub shard_mailbox_capacity: u64,
    /// Fraction (0.0..=1.0) of `shard_mailbox_capacity` above which the runtime
    /// reports degraded.
    pub degraded_mailbox_fraction: f64,
    /// Timer lag in milliseconds above which the runtime reports degraded.
    pub degraded_timer_lag_ms: u64,
    /// Timer lag in milliseconds above which the runtime reports not-ready.
    pub not_ready_timer_lag_ms: u64,
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            shard_mailbox_capacity: 8192,
            degraded_mailbox_fraction: 0.8,
            degraded_timer_lag_ms: 1_000,
            not_ready_timer_lag_ms: 10_000,
        }
    }
}

impl HealthThresholds {
    /// Creates thresholds derived from a shard mailbox capacity, using default
    /// degraded/not-ready ratios and timer-lag limits.
    pub fn from_mailbox_capacity(capacity: u64) -> Self {
        Self {
            shard_mailbox_capacity: capacity.max(1),
            ..Self::default()
        }
    }

    fn degraded_mailbox_depth(&self) -> u64 {
        let fraction = self.degraded_mailbox_fraction.clamp(0.0, 1.0);
        ((self.shard_mailbox_capacity as f64) * fraction).ceil() as u64
    }
}

/// A bounded, fixed set of reasons a runtime is degraded or not-ready.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthReason {
    /// One or more shard mailboxes exceeded the degraded depth threshold.
    MailboxPressure,
    /// A shard mailbox reached full capacity.
    MailboxSaturated,
    /// Timer-wheel lag exceeded the degraded threshold.
    TimerLag,
    /// Timer-wheel lag exceeded the not-ready threshold.
    TimerLagCritical,
}

impl HealthReason {
    /// Stable, low-cardinality string form.
    pub const fn as_str(self) -> &'static str {
        match self {
            HealthReason::MailboxPressure => "mailbox_pressure",
            HealthReason::MailboxSaturated => "mailbox_saturated",
            HealthReason::TimerLag => "timer_lag",
            HealthReason::TimerLagCritical => "timer_lag_critical",
        }
    }
}

/// Bounded snapshot of runtime readiness and pressure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeHealth {
    /// True unless the runtime is too saturated to accept new work.
    pub ready: bool,
    /// True when the runtime is serving but under sustained pressure.
    pub degraded: bool,
    /// Fixed reason codes explaining a degraded/not-ready state.
    pub reasons: Vec<HealthReason>,
    /// Highest observed shard mailbox depth across all shards.
    pub max_shard_mailbox_depth: u64,
    /// Currently loaded actors.
    pub active_actors: u64,
    /// Most recent timer-wheel tick lag in milliseconds.
    pub timer_lag_ms: u64,
}

impl RuntimeHealth {
    /// A fully healthy snapshot with no observed pressure.
    pub fn healthy() -> Self {
        Self {
            ready: true,
            degraded: false,
            reasons: Vec::new(),
            max_shard_mailbox_depth: 0,
            active_actors: 0,
            timer_lag_ms: 0,
        }
    }

    /// Classifies observed runtime state against `thresholds`.
    pub fn evaluate(
        thresholds: &HealthThresholds,
        max_shard_mailbox_depth: u64,
        active_actors: u64,
        timer_lag_ms: u64,
    ) -> Self {
        let mut reasons = Vec::new();
        let mut ready = true;

        if max_shard_mailbox_depth >= thresholds.shard_mailbox_capacity {
            reasons.push(HealthReason::MailboxSaturated);
            ready = false;
        } else if max_shard_mailbox_depth >= thresholds.degraded_mailbox_depth() {
            reasons.push(HealthReason::MailboxPressure);
        }

        if timer_lag_ms >= thresholds.not_ready_timer_lag_ms {
            reasons.push(HealthReason::TimerLagCritical);
            ready = false;
        } else if timer_lag_ms >= thresholds.degraded_timer_lag_ms {
            reasons.push(HealthReason::TimerLag);
        }

        Self {
            ready,
            degraded: !reasons.is_empty(),
            reasons,
            max_shard_mailbox_depth,
            active_actors,
            timer_lag_ms,
        }
    }
}

/// A source of the current [`RuntimeHealth`] snapshot.
///
/// Implemented by the metrics aggregator so transport adapters (e.g. the HTTP
/// readiness endpoint) can report runtime health without depending on the
/// concrete runtime internals.
pub trait RuntimeHealthSource: Send + Sync {
    /// Returns the current runtime health snapshot.
    fn runtime_health(&self) -> RuntimeHealth;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thresholds() -> HealthThresholds {
        HealthThresholds {
            shard_mailbox_capacity: 100,
            degraded_mailbox_fraction: 0.8,
            degraded_timer_lag_ms: 1_000,
            not_ready_timer_lag_ms: 10_000,
        }
    }

    #[test]
    fn healthy_when_below_all_thresholds() {
        let health = RuntimeHealth::evaluate(&thresholds(), 10, 5, 100);
        assert!(health.ready);
        assert!(!health.degraded);
        assert!(health.reasons.is_empty());
    }

    #[test]
    fn degraded_on_mailbox_pressure_but_still_ready() {
        let health = RuntimeHealth::evaluate(&thresholds(), 85, 5, 0);
        assert!(health.ready);
        assert!(health.degraded);
        assert_eq!(health.reasons, vec![HealthReason::MailboxPressure]);
    }

    #[test]
    fn not_ready_when_mailbox_saturated() {
        let health = RuntimeHealth::evaluate(&thresholds(), 100, 5, 0);
        assert!(!health.ready);
        assert!(health.degraded);
        assert!(health.reasons.contains(&HealthReason::MailboxSaturated));
    }

    #[test]
    fn timer_lag_escalates_from_degraded_to_not_ready() {
        let degraded = RuntimeHealth::evaluate(&thresholds(), 0, 0, 2_000);
        assert!(degraded.ready);
        assert!(degraded.degraded);
        assert_eq!(degraded.reasons, vec![HealthReason::TimerLag]);

        let critical = RuntimeHealth::evaluate(&thresholds(), 0, 0, 20_000);
        assert!(!critical.ready);
        assert!(critical.reasons.contains(&HealthReason::TimerLagCritical));
    }

    #[test]
    fn from_mailbox_capacity_uses_defaults() {
        let t = HealthThresholds::from_mailbox_capacity(4096);
        assert_eq!(t.shard_mailbox_capacity, 4096);
        assert_eq!(t.degraded_mailbox_depth(), (4096.0 * 0.8_f64).ceil() as u64);
    }
}

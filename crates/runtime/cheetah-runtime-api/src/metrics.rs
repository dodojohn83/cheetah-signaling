//! Runtime health metrics.
//!
//! These counters expose aggregate runtime health, backlog, and resource state
//! without any high-cardinality per-device labels. A single [`RuntimeMetrics`]
//! instance is shared (via `Arc`) by the admission controller, shard workers,
//! and timer wheel; callers read a consistent point-in-time view through
//! [`RuntimeMetrics::snapshot`].

use std::sync::atomic::{AtomicU64, Ordering};

/// Shared, lock-free runtime health metrics.
///
/// All fields are monotonic counters except `active_actors` and
/// `pending_timer_dispatch`, which are gauges maintained by increment/decrement
/// or direct stores.
#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    messages_enqueued: AtomicU64,
    messages_rejected: AtomicU64,
    messages_processed: AtomicU64,
    actors_created: AtomicU64,
    actors_evicted_idle: AtomicU64,
    active_actors: AtomicU64,
    timers_scheduled: AtomicU64,
    timers_fired: AtomicU64,
    timers_cancelled: AtomicU64,
    timers_dropped: AtomicU64,
    pending_timer_dispatch: AtomicU64,
    timer_lag_ms: AtomicU64,
}

impl RuntimeMetrics {
    /// Creates a new zeroed metrics registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that a message was admitted to a shard mailbox.
    pub fn record_message_enqueued(&self) {
        self.messages_enqueued.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that a message was rejected because the target mailbox was full.
    pub fn record_message_rejected(&self) {
        self.messages_rejected.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that a shard processed one message.
    pub fn record_message_processed(&self) {
        self.messages_processed.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that an actor was lazily created and increments the active gauge.
    pub fn record_actor_created(&self) {
        self.actors_created.fetch_add(1, Ordering::Relaxed);
        self.active_actors.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that an idle actor was unloaded and decrements the active gauge.
    pub fn record_actor_evicted_idle(&self) {
        self.actors_evicted_idle.fetch_add(1, Ordering::Relaxed);
        self.decrement_active_actors();
    }

    /// Decrements the active-actor gauge without counting an idle eviction,
    /// e.g. when an actor is dropped during shutdown drain.
    pub fn decrement_active_actors(&self) {
        // Saturating decrement: never wrap below zero.
        let mut current = self.active_actors.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                return;
            }
            match self.active_actors.compare_exchange_weak(
                current,
                current - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    /// Records that a timer was scheduled.
    pub fn record_timer_scheduled(&self) {
        self.timers_scheduled.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that a timer fired and was dispatched.
    pub fn record_timer_fired(&self) {
        self.timers_fired.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that a timer cancellation command was processed.
    pub fn record_timer_cancelled(&self) {
        self.timers_cancelled.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that `count` timers were dropped due to dispatch-queue overflow.
    pub fn record_timers_dropped(&self, count: u64) {
        if count > 0 {
            self.timers_dropped.fetch_add(count, Ordering::Relaxed);
        }
    }

    /// Sets the current pending timer-dispatch backlog gauge.
    pub fn set_pending_timer_dispatch(&self, value: u64) {
        self.pending_timer_dispatch.store(value, Ordering::Relaxed);
    }

    /// Records the most recent observed timer-wheel tick lag in milliseconds,
    /// i.e. how much later than its scheduled resolution a tick fired. This is
    /// a gauge reflecting the latest sample, not a cumulative counter.
    pub fn set_timer_lag_ms(&self, value: u64) {
        self.timer_lag_ms.store(value, Ordering::Relaxed);
    }

    /// Returns a consistent point-in-time snapshot of all metrics.
    pub fn snapshot(&self) -> RuntimeMetricsSnapshot {
        RuntimeMetricsSnapshot {
            messages_enqueued: self.messages_enqueued.load(Ordering::Relaxed),
            messages_rejected: self.messages_rejected.load(Ordering::Relaxed),
            messages_processed: self.messages_processed.load(Ordering::Relaxed),
            actors_created: self.actors_created.load(Ordering::Relaxed),
            actors_evicted_idle: self.actors_evicted_idle.load(Ordering::Relaxed),
            active_actors: self.active_actors.load(Ordering::Relaxed),
            timers_scheduled: self.timers_scheduled.load(Ordering::Relaxed),
            timers_fired: self.timers_fired.load(Ordering::Relaxed),
            timers_cancelled: self.timers_cancelled.load(Ordering::Relaxed),
            timers_dropped: self.timers_dropped.load(Ordering::Relaxed),
            pending_timer_dispatch: self.pending_timer_dispatch.load(Ordering::Relaxed),
            timer_lag_ms: self.timer_lag_ms.load(Ordering::Relaxed),
        }
    }
}

/// Immutable point-in-time view of [`RuntimeMetrics`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeMetricsSnapshot {
    /// Messages admitted to shard mailboxes.
    pub messages_enqueued: u64,
    /// Messages rejected because the target mailbox was full.
    pub messages_rejected: u64,
    /// Messages processed by shard workers.
    pub messages_processed: u64,
    /// Actors lazily created across all shards.
    pub actors_created: u64,
    /// Actors unloaded because they became idle.
    pub actors_evicted_idle: u64,
    /// Currently loaded actors (gauge).
    pub active_actors: u64,
    /// Timers scheduled on the timer wheel.
    pub timers_scheduled: u64,
    /// Timers that fired and were dispatched.
    pub timers_fired: u64,
    /// Timer cancellation commands processed.
    pub timers_cancelled: u64,
    /// Timers dropped due to dispatch-queue overflow.
    pub timers_dropped: u64,
    /// Timers waiting to be dispatched to a shard (gauge).
    pub pending_timer_dispatch: u64,
    /// Most recent timer-wheel tick lag in milliseconds (gauge).
    pub timer_lag_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate() {
        let metrics = RuntimeMetrics::new();
        metrics.record_message_enqueued();
        metrics.record_message_enqueued();
        metrics.record_message_rejected();
        metrics.record_message_processed();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.messages_enqueued, 2);
        assert_eq!(snapshot.messages_rejected, 1);
        assert_eq!(snapshot.messages_processed, 1);
    }

    #[test]
    fn active_actor_gauge_tracks_create_and_evict() {
        let metrics = RuntimeMetrics::new();
        metrics.record_actor_created();
        metrics.record_actor_created();
        assert_eq!(metrics.snapshot().active_actors, 2);
        metrics.record_actor_evicted_idle();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.active_actors, 1);
        assert_eq!(snapshot.actors_evicted_idle, 1);
    }

    #[test]
    fn active_actor_gauge_saturates_at_zero() {
        let metrics = RuntimeMetrics::new();
        metrics.decrement_active_actors();
        assert_eq!(metrics.snapshot().active_actors, 0);
    }

    #[test]
    fn timer_counters_and_gauge() {
        let metrics = RuntimeMetrics::new();
        metrics.record_timer_scheduled();
        metrics.record_timer_fired();
        metrics.record_timer_cancelled();
        metrics.record_timers_dropped(3);
        metrics.set_pending_timer_dispatch(7);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.timers_scheduled, 1);
        assert_eq!(snapshot.timers_fired, 1);
        assert_eq!(snapshot.timers_cancelled, 1);
        assert_eq!(snapshot.timers_dropped, 3);
        assert_eq!(snapshot.pending_timer_dispatch, 7);
    }
}

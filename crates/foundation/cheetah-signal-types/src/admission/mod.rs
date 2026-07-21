//! Reusable, deterministic admission-control primitives.
//!
//! This module provides small, pure building blocks used by both the runtime
//! and the application layers to implement admission control, priority routing,
//! coalescing, dead-lettering and backlog recovery:
//!
//! * [`TrafficClass`] / [`Priority`] classify inbound work.
//! * [`TokenBucket`] / [`KeyedRateLimiter`] enforce bounded per-key rate limits.
//! * [`Coalescer`] collapses redundant, still-pending events.
//! * [`DeadLetterQueue`] holds rejected work for bounded redrive.
//! * [`BacklogController`] tracks overload with hysteresis.
//!
//! All types are synchronous, deterministic and driven by an externally
//! supplied monotonic millisecond clock, so they contain no I/O, no async and
//! no unbounded state. This keeps them usable from `domain`/foundation code
//! without violating the layering rules.

mod backlog;
mod coalescer;
mod dead_letter;
mod priority;
mod rate_limiter;
mod token_bucket;
mod traffic_class;

pub use backlog::{BacklogController, BacklogObservation, BacklogState};
pub use coalescer::{CoalesceDecision, Coalescer};
pub use dead_letter::{DeadLetterEntry, DeadLetterQueue, DeadLetterReason};
pub use priority::Priority;
pub use rate_limiter::KeyedRateLimiter;
pub use token_bucket::{TokenBucket, TokenBucketConfig};
pub use traffic_class::TrafficClass;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_enforces_capacity_and_refill() {
        let config = TokenBucketConfig {
            capacity_tokens: 2,
            refill_tokens_per_sec: 1,
        };
        let mut bucket = TokenBucket::new(config, 0);
        assert!(bucket.try_acquire(0, 1));
        assert!(bucket.try_acquire(0, 1));
        // Bucket empty now.
        assert!(!bucket.try_acquire(0, 1));
        // One token refills after 1000ms.
        assert!(bucket.try_acquire(1_000, 1));
        assert!(!bucket.try_acquire(1_000, 1));
    }

    #[test]
    fn token_bucket_ignores_backwards_time() {
        let config = TokenBucketConfig {
            capacity_tokens: 1,
            refill_tokens_per_sec: 1_000,
        };
        let mut bucket = TokenBucket::new(config, 100);
        assert!(bucket.try_acquire(100, 1));
        // Time goes backwards: no refill, still empty.
        assert!(!bucket.try_acquire(50, 1));
    }

    #[test]
    fn keyed_rate_limiter_bounds_keys_via_lru() {
        let config = TokenBucketConfig {
            capacity_tokens: 1,
            refill_tokens_per_sec: 1,
        };
        let mut limiter = KeyedRateLimiter::new(config, 2);
        assert!(limiter.try_acquire("a", 0));
        assert!(limiter.try_acquire("b", 0));
        assert_eq!(limiter.tracked_keys(), 2);
        // Third distinct key evicts the LRU ("a").
        assert!(limiter.try_acquire("c", 0));
        assert_eq!(limiter.tracked_keys(), 2);
        assert!(limiter.evicted_total() >= 1);
    }

    #[test]
    fn keyed_rate_limiter_limits_per_key() {
        let config = TokenBucketConfig {
            capacity_tokens: 1,
            refill_tokens_per_sec: 1,
        };
        let mut limiter = KeyedRateLimiter::new(config, 8);
        assert!(limiter.try_acquire("a", 0));
        assert!(!limiter.try_acquire("a", 0));
        // Different key has its own bucket.
        assert!(limiter.try_acquire("b", 0));
    }

    #[test]
    fn coalescer_collapses_pending_and_releases() {
        let mut coalescer = Coalescer::new(8);
        assert_eq!(coalescer.observe("k"), CoalesceDecision::Admit);
        assert_eq!(coalescer.observe("k"), CoalesceDecision::Coalesced);
        assert_eq!(coalescer.observe("k"), CoalesceDecision::Coalesced);
        assert_eq!(coalescer.coalesced_total(), 2);
        coalescer.release(&"k");
        assert_eq!(coalescer.observe("k"), CoalesceDecision::Admit);
    }

    #[test]
    fn dead_letter_queue_is_bounded_and_redrivable() {
        let mut dlq = DeadLetterQueue::new(2);
        dlq.push(1, DeadLetterReason::Overloaded, 0);
        dlq.push(2, DeadLetterReason::Overloaded, 0);
        dlq.push(3, DeadLetterReason::Overloaded, 0);
        assert_eq!(dlq.len(), 2);
        assert_eq!(dlq.dropped_total(), 1);
        let drained = dlq.drain(10);
        // Oldest (1) was dropped, so 2 and 3 remain.
        assert_eq!(
            drained.iter().map(|e| e.payload).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert!(dlq.is_empty());
        assert_eq!(dlq.redriven_total(), 2);
    }

    #[test]
    fn backlog_controller_has_hysteresis() {
        let mut controller = BacklogController::new(10, 3);
        assert!(!controller.observe(5).entered_overload);
        let hit = controller.observe(10);
        assert!(hit.entered_overload);
        assert!(controller.shed_low_priority());
        // Still overloaded between watermarks.
        assert!(!controller.observe(4).recovered);
        assert!(controller.shed_low_priority());
        let recovered = controller.observe(3);
        assert!(recovered.recovered);
        assert!(!controller.shed_low_priority());
        assert_eq!(controller.overload_transitions(), 1);
        assert_eq!(controller.recovery_transitions(), 1);
    }

    #[test]
    fn traffic_class_priorities() {
        assert_eq!(TrafficClass::Command.priority(), Priority::High);
        assert_eq!(TrafficClass::Keepalive.priority(), Priority::Low);
        assert!(TrafficClass::Keepalive.is_coalescible());
        assert!(TrafficClass::Position.is_coalescible());
        assert!(!TrafficClass::Command.is_coalescible());
        assert!(Priority::Low < Priority::High);
    }
}

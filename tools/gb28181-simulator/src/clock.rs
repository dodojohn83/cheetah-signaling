//! Deterministic virtual clock and hierarchical timer wheel.
//!
//! The simulator is a discrete-event simulation: instead of one Tokio task and
//! sleep per device, a single fixed set of shard workers pull timers from a
//! shared, monotonically ordered timer wheel.  Ordering is fully deterministic:
//! events sort by `(due_ms, sequence)` where `sequence` is a monotonic counter,
//! so a given seed and scenario always replay identically.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Monotonic virtual clock measured in milliseconds from run start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualClock {
    now_ms: u64,
}

impl VirtualClock {
    /// Current virtual time in milliseconds.
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Advances the clock to `to_ms`, which must not move time backwards.
    pub fn advance_to(&mut self, to_ms: u64) {
        debug_assert!(to_ms >= self.now_ms, "virtual clock must be monotonic");
        if to_ms > self.now_ms {
            self.now_ms = to_ms;
        }
    }
}

/// A scheduled entry in the timer wheel.
#[derive(Clone, Debug)]
struct Scheduled<E> {
    due_ms: u64,
    sequence: u64,
    event: E,
}

impl<E> PartialEq for Scheduled<E> {
    fn eq(&self, other: &Self) -> bool {
        self.due_ms == other.due_ms && self.sequence == other.sequence
    }
}

impl<E> Eq for Scheduled<E> {}

impl<E> PartialOrd for Scheduled<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<E> Ord for Scheduled<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering so the `BinaryHeap` behaves as a min-heap on
        // `(due_ms, sequence)`.
        other
            .due_ms
            .cmp(&self.due_ms)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

/// Deterministic min-heap timer wheel over generic events.
#[derive(Debug)]
pub struct TimerWheel<E> {
    clock: VirtualClock,
    heap: BinaryHeap<Scheduled<E>>,
    next_sequence: u64,
    peak_len: usize,
    processed: u64,
}

impl<E> Default for TimerWheel<E> {
    fn default() -> Self {
        Self {
            clock: VirtualClock::default(),
            heap: BinaryHeap::new(),
            next_sequence: 0,
            peak_len: 0,
            processed: 0,
        }
    }
}

impl<E> TimerWheel<E> {
    /// Creates an empty timer wheel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current virtual time in milliseconds.
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Schedules `event` to fire at absolute virtual time `due_ms`.
    ///
    /// A `due_ms` in the past is clamped to now, preserving monotonicity.
    pub fn schedule(&mut self, due_ms: u64, event: E) {
        let due_ms = due_ms.max(self.clock.now_ms());
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.heap.push(Scheduled {
            due_ms,
            sequence,
            event,
        });
        self.peak_len = self.peak_len.max(self.heap.len());
    }

    /// Schedules `event` to fire `delay_ms` after the current virtual time.
    pub fn schedule_after(&mut self, delay_ms: u64, event: E) {
        self.schedule(self.clock.now_ms().saturating_add(delay_ms), event);
    }

    /// Pops the next due event, advancing the clock to its due time.
    ///
    /// Returns `None` when the wheel is empty.
    pub fn pop(&mut self) -> Option<(u64, E)> {
        let Scheduled { due_ms, event, .. } = self.heap.pop()?;
        self.clock.advance_to(due_ms);
        self.processed += 1;
        Some((due_ms, event))
    }

    /// Number of events currently queued.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Whether the wheel is empty.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Peak number of simultaneously queued events observed so far.
    pub fn peak_len(&self) -> usize {
        self.peak_len
    }

    /// Total number of events popped and processed so far.
    pub fn processed(&self) -> u64 {
        self.processed
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn pops_in_time_then_sequence_order() {
        let mut wheel: TimerWheel<&str> = TimerWheel::new();
        wheel.schedule(30, "c");
        wheel.schedule(10, "a");
        wheel.schedule(10, "b");
        wheel.schedule(20, "later-but-same-as-earlier");

        let mut order = Vec::new();
        while let Some((t, e)) = wheel.pop() {
            order.push((t, e));
        }
        assert_eq!(
            order,
            vec![
                (10, "a"),
                (10, "b"),
                (20, "later-but-same-as-earlier"),
                (30, "c")
            ]
        );
    }

    #[test]
    fn schedule_after_uses_current_time() {
        let mut wheel: TimerWheel<u32> = TimerWheel::new();
        wheel.schedule(100, 1);
        let (_t, _e) = wheel.pop().unwrap();
        assert_eq!(wheel.now_ms(), 100);
        wheel.schedule_after(50, 2);
        let (t, e) = wheel.pop().unwrap();
        assert_eq!((t, e), (150, 2));
    }

    #[test]
    fn tracks_peak_and_processed() {
        let mut wheel: TimerWheel<u8> = TimerWheel::new();
        wheel.schedule(1, 0);
        wheel.schedule(2, 0);
        wheel.schedule(3, 0);
        assert_eq!(wheel.peak_len(), 3);
        wheel.pop();
        wheel.pop();
        assert_eq!(wheel.processed(), 2);
        assert_eq!(wheel.len(), 1);
    }
}

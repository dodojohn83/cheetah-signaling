//! Bounded dead-letter queue with redrive support.

use std::collections::VecDeque;

/// Reason a unit of work was dead-lettered. The set is fixed and bounded so it
/// is safe to use as a metrics label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeadLetterReason {
    /// Rejected by a rate limiter.
    RateLimited,
    /// Dropped because the target was overloaded.
    Overloaded,
    /// Shed because it was low priority during overload.
    PriorityShed,
    /// Failed to be redriven within the retry budget.
    RedriveExhausted,
}

impl DeadLetterReason {
    /// Returns a stable, bounded string label suitable for metrics.
    pub const fn as_str(self) -> &'static str {
        match self {
            DeadLetterReason::RateLimited => "rate_limited",
            DeadLetterReason::Overloaded => "overloaded",
            DeadLetterReason::PriorityShed => "priority_shed",
            DeadLetterReason::RedriveExhausted => "redrive_exhausted",
        }
    }
}

/// A single dead-lettered entry carrying its payload and diagnostic metadata.
#[derive(Clone, Debug)]
pub struct DeadLetterEntry<T> {
    /// The dead-lettered payload.
    pub payload: T,
    /// Why the payload was dead-lettered.
    pub reason: DeadLetterReason,
    /// Monotonic time (ms) at which the entry was enqueued.
    pub enqueued_at_ms: i64,
    /// Number of redrive attempts already made for this payload.
    pub attempts: u32,
}

/// A bounded FIFO dead-letter queue.
///
/// When full, the oldest entry is dropped to make room, keeping memory
/// bounded. Entries can be drained in batches for redrive.
#[derive(Debug)]
pub struct DeadLetterQueue<T> {
    buf: VecDeque<DeadLetterEntry<T>>,
    capacity: usize,
    enqueued_total: u64,
    dropped_total: u64,
    redriven_total: u64,
}

impl<T> DeadLetterQueue<T> {
    /// Creates a queue holding at most `capacity` entries (clamped to >= 1).
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            capacity: capacity.max(1),
            enqueued_total: 0,
            dropped_total: 0,
            redriven_total: 0,
        }
    }

    /// Enqueues a fresh payload with zero prior attempts.
    pub fn push(&mut self, payload: T, reason: DeadLetterReason, now_ms: i64) {
        self.push_entry(DeadLetterEntry {
            payload,
            reason,
            enqueued_at_ms: now_ms,
            attempts: 0,
        });
    }

    /// Enqueues a pre-built entry (used when re-dead-lettering after a failed
    /// redrive so the attempt count is preserved).
    pub fn push_entry(&mut self, entry: DeadLetterEntry<T>) {
        if self.buf.len() >= self.capacity {
            self.buf.pop_front();
            self.dropped_total += 1;
        }
        self.buf.push_back(entry);
        self.enqueued_total += 1;
    }

    /// Removes and returns up to `max` entries from the front for redrive.
    pub fn drain(&mut self, max: usize) -> Vec<DeadLetterEntry<T>> {
        let take = max.min(self.buf.len());
        let mut out = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(entry) = self.buf.pop_front() {
                out.push(entry);
            }
        }
        self.redriven_total += out.len() as u64;
        out
    }

    /// Returns the number of entries currently queued.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns `true` when the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Returns the configured capacity.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Total entries ever enqueued.
    pub const fn enqueued_total(&self) -> u64 {
        self.enqueued_total
    }

    /// Total entries dropped due to the capacity bound.
    pub const fn dropped_total(&self) -> u64 {
        self.dropped_total
    }

    /// Total entries drained for redrive.
    pub const fn redriven_total(&self) -> u64 {
        self.redriven_total
    }
}

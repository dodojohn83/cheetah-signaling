//! Bounded coalescer for redundant, pending events.

use std::collections::HashSet;
use std::hash::Hash;

/// Decision returned by [`Coalescer::observe`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoalesceDecision {
    /// The event should be admitted; no equivalent event is currently pending.
    Admit,
    /// An equivalent event is already pending and this one was coalesced away.
    Coalesced,
}

/// Tracks which keys currently have a pending (admitted but not yet released)
/// coalescible event, so redundant events for the same key collapse into the
/// one already in flight.
///
/// The number of tracked keys is bounded; once the bound is reached new keys
/// are admitted but not tracked (so they are never dropped, they simply cannot
/// coalesce until capacity frees up). Callers must invoke [`Coalescer::release`]
/// once a pending event has been processed.
#[derive(Debug)]
pub struct Coalescer<K: Eq + Hash + Clone> {
    pending: HashSet<K>,
    max_tracked: usize,
    coalesced_total: u64,
}

impl<K: Eq + Hash + Clone> Coalescer<K> {
    /// Creates a new coalescer tracking at most `max_tracked` keys.
    pub fn new(max_tracked: usize) -> Self {
        Self {
            pending: HashSet::new(),
            max_tracked: max_tracked.max(1),
            coalesced_total: 0,
        }
    }

    /// Records observation of a coalescible event for `key`.
    pub fn observe(&mut self, key: K) -> CoalesceDecision {
        if self.pending.contains(&key) {
            self.coalesced_total += 1;
            CoalesceDecision::Coalesced
        } else {
            if self.pending.len() < self.max_tracked {
                self.pending.insert(key);
            }
            CoalesceDecision::Admit
        }
    }

    /// Marks the pending event for `key` as processed.
    pub fn release(&mut self, key: &K) {
        self.pending.remove(key);
    }

    /// Returns the number of keys with a pending event.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Returns the total number of coalesced (dropped) events.
    pub const fn coalesced_total(&self) -> u64 {
        self.coalesced_total
    }
}

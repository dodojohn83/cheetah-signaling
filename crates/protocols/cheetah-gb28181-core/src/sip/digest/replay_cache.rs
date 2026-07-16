//! Bounded replay cache for digest `nc` values.

use std::collections::VecDeque;

/// Bounded replay cache for digest `nc` values.
///
/// Per RFC 2617 §3.2.2, the server must ensure that the nonce-count for a given
/// nonce is strictly increasing. This implementation tracks the highest `nc`
/// seen for each nonce and rejects any request with an `nc` that is not greater
/// than the recorded maximum.
#[derive(Debug)]
pub struct DigestReplayCache {
    capacity: usize,
    entries: VecDeque<ReplayEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayEntry {
    nonce: String,
    max_nc: u64,
    inserted_at: u64,
}

impl DigestReplayCache {
    /// Creates a cache with the given maximum number of stored nonce entries.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity.min(1024)),
        }
    }

    /// Records the `nc` value and returns `true` if it is new, `false` if it
    /// has been seen before or is not strictly increasing for the nonce.
    pub fn check(&mut self, nonce: &str, nc: u64, now: u64, ttl: u64) -> bool {
        self.prune(now, ttl);

        for e in &mut self.entries {
            if e.nonce == nonce {
                if nc > e.max_nc {
                    e.max_nc = nc;
                    return true;
                }
                return false;
            }
        }

        self.entries.push_back(ReplayEntry {
            nonce: nonce.to_string(),
            max_nc: nc,
            inserted_at: now,
        });
        if self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
        true
    }

    /// Removes entries older than `ttl` seconds relative to `now`.
    pub fn prune(&mut self, now: u64, ttl: u64) {
        while self
            .entries
            .front()
            .is_some_and(|e| e.inserted_at.saturating_add(ttl) < now)
        {
            self.entries.pop_front();
        }
    }
}

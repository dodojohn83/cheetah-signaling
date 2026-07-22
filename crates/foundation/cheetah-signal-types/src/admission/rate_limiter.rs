//! Bounded, keyed token-bucket rate limiter.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

use super::token_bucket::{TokenBucket, TokenBucketConfig};

/// A rate limiter that maintains one [`TokenBucket`] per key with a hard bound
/// on the number of tracked keys.
///
/// When the key limit is reached, the least-recently-used key is evicted. This
/// keeps memory bounded regardless of how many distinct sources or methods are
/// observed, in line with the workspace requirement that all caches be bounded.
#[derive(Debug)]
pub struct KeyedRateLimiter<K: Eq + Hash + Clone> {
    config: TokenBucketConfig,
    max_keys: usize,
    buckets: HashMap<K, TokenBucket>,
    lru: VecDeque<K>,
    evicted_total: u64,
}

impl<K: Eq + Hash + Clone> KeyedRateLimiter<K> {
    /// Creates a new limiter. `max_keys` is clamped to at least one.
    pub fn new(config: TokenBucketConfig, max_keys: usize) -> Self {
        Self {
            config: config.sanitized(),
            max_keys: max_keys.max(1),
            buckets: HashMap::new(),
            lru: VecDeque::new(),
            evicted_total: 0,
        }
    }

    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.lru.iter().position(|k| k == key) {
            self.lru.remove(pos);
        }
        self.lru.push_back(key.clone());
    }

    fn evict_if_needed(&mut self) {
        while self.buckets.len() >= self.max_keys {
            match self.lru.pop_front() {
                Some(oldest) => {
                    if self.buckets.remove(&oldest).is_some() {
                        self.evicted_total += 1;
                    }
                }
                None => break,
            }
        }
    }

    /// Attempts to consume one token for `key` at `now_ms`.
    ///
    /// Returns `true` when the request is within the rate limit.
    pub fn try_acquire(&mut self, key: K, now_ms: i64) -> bool {
        if self.buckets.contains_key(&key) {
            self.touch(&key);
            // Unwrap-free access: presence checked above.
            match self.buckets.get_mut(&key) {
                Some(bucket) => bucket.try_acquire(now_ms, 1),
                None => false,
            }
        } else {
            self.evict_if_needed();
            let mut bucket = TokenBucket::new(self.config, now_ms);
            let allowed = bucket.try_acquire(now_ms, 1);
            self.buckets.insert(key.clone(), bucket);
            self.lru.push_back(key);
            allowed
        }
    }

    /// Returns the number of currently tracked keys.
    pub fn tracked_keys(&self) -> usize {
        self.buckets.len()
    }

    /// Returns the total number of keys evicted due to the bound.
    pub const fn evicted_total(&self) -> u64 {
        self.evicted_total
    }
}

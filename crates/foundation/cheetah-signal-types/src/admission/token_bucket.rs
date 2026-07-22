//! Deterministic, monotonic-time token bucket.

/// Number of milli-tokens per whole token.
const MILLI: u64 = 1_000;

/// Configuration for a [`TokenBucket`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenBucketConfig {
    /// Maximum number of whole tokens the bucket can hold (burst size).
    pub capacity_tokens: u64,
    /// Sustained refill rate in tokens per second.
    pub refill_tokens_per_sec: u64,
}

impl TokenBucketConfig {
    /// Returns a configuration with both fields clamped to at least one so a
    /// bucket never becomes permanently empty due to a zero rate or capacity.
    pub const fn sanitized(self) -> Self {
        Self {
            capacity_tokens: if self.capacity_tokens == 0 {
                1
            } else {
                self.capacity_tokens
            },
            refill_tokens_per_sec: if self.refill_tokens_per_sec == 0 {
                1
            } else {
                self.refill_tokens_per_sec
            },
        }
    }
}

/// A single token bucket rate limiter driven by an externally supplied
/// monotonic millisecond clock.
///
/// The bucket tracks fractional tokens internally (as milli-tokens) so that
/// low refill rates still make steady progress between calls. All arithmetic
/// is saturating and time going backwards is treated as no elapsed time, so
/// the bucket is fully deterministic and panic-free.
#[derive(Clone, Copy, Debug)]
pub struct TokenBucket {
    capacity_milli: u64,
    refill_per_sec: u64,
    tokens_milli: u64,
    last_refill_ms: i64,
}

impl TokenBucket {
    /// Creates a full bucket as of `now_ms`.
    pub fn new(config: TokenBucketConfig, now_ms: i64) -> Self {
        let config = config.sanitized();
        let capacity_milli = config.capacity_tokens.saturating_mul(MILLI);
        Self {
            capacity_milli,
            refill_per_sec: config.refill_tokens_per_sec,
            tokens_milli: capacity_milli,
            last_refill_ms: now_ms,
        }
    }

    fn refill(&mut self, now_ms: i64) {
        let elapsed_ms = now_ms.saturating_sub(self.last_refill_ms);
        if elapsed_ms <= 0 {
            return;
        }
        // `refill_per_sec` tokens/sec equals `refill_per_sec` milli-tokens/ms.
        let added = (elapsed_ms as u64).saturating_mul(self.refill_per_sec);
        self.tokens_milli = self
            .tokens_milli
            .saturating_add(added)
            .min(self.capacity_milli);
        self.last_refill_ms = now_ms;
    }

    /// Attempts to consume `cost_tokens` whole tokens, refilling first.
    ///
    /// Returns `true` when enough tokens were available and consumed.
    pub fn try_acquire(&mut self, now_ms: i64, cost_tokens: u64) -> bool {
        self.refill(now_ms);
        let cost_milli = cost_tokens.max(1).saturating_mul(MILLI);
        if self.tokens_milli >= cost_milli {
            self.tokens_milli -= cost_milli;
            true
        } else {
            false
        }
    }

    /// Returns the currently available whole tokens (for tests and metrics).
    pub const fn available_tokens(&self) -> u64 {
        self.tokens_milli / MILLI
    }
}

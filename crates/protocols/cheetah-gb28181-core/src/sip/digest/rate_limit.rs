//! Bounded per-source authentication rate limiter.
//!
//! Mitigates online brute-force attacks against digest authentication by
//! counting failed authentication attempts per source IP within a sliding
//! window. When a source exceeds the configured failure budget it is blocked
//! until the window elapses. The limiter is Sans-I/O and time-injected: callers
//! pass a monotonic-ish `now` (in seconds), so behaviour is deterministic in
//! tests.
//!
//! All state is bounded: the number of tracked sources is capped and the oldest
//! tracked source is evicted (FIFO) when the capacity is reached. This satisfies
//! the "no unbounded caches" requirement for authentication-facing state.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::net::IpAddr;

/// Per-source failure window used to enforce a brute-force budget.
#[derive(Clone, Copy, Debug)]
struct FailureWindow {
    /// Number of failures observed in the current window.
    failures: u32,
    /// Second at which the current window started.
    window_start: u64,
}

/// Bounded per-source authentication rate limiter.
///
/// A source is *blocked* once it accumulates `max_failures` failed
/// authentication attempts within `window_seconds`. The block is released when
/// the window expires. A successful authentication clears the source's failure
/// state immediately, so legitimate devices that eventually authenticate are
/// never penalised.
///
/// Setting `max_failures` or `max_sources` to zero disables the limiter (it
/// never blocks). This lets deployments opt out without special-casing at the
/// call site.
#[derive(Debug)]
pub struct AuthRateLimiter {
    max_failures: u32,
    window_seconds: u64,
    max_sources: usize,
    entries: HashMap<IpAddr, FailureWindow>,
    order: VecDeque<IpAddr>,
}

impl AuthRateLimiter {
    /// Creates a limiter allowing `max_failures` failures per `window_seconds`
    /// for each source, tracking at most `max_sources` distinct sources.
    pub fn new(max_failures: u32, window_seconds: u64, max_sources: usize) -> Self {
        Self {
            max_failures,
            window_seconds,
            max_sources,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns `true` when the limiter is effectively disabled.
    fn disabled(&self) -> bool {
        self.max_failures == 0 || self.max_sources == 0 || self.window_seconds == 0
    }

    /// Returns whether `source` is currently blocked at `now`.
    ///
    /// A source is blocked when it has reached the failure budget and its
    /// window has not yet expired.
    pub fn is_blocked(&self, source: IpAddr, now: u64) -> bool {
        if self.disabled() {
            return false;
        }
        match self.entries.get(&source) {
            Some(window) => {
                let expired = now.saturating_sub(window.window_start) >= self.window_seconds;
                !expired && window.failures >= self.max_failures
            }
            None => false,
        }
    }

    /// Returns the number of seconds until `source`'s block is released.
    ///
    /// Returns `0` when the source is not currently blocked.
    pub fn retry_after_seconds(&self, source: IpAddr, now: u64) -> u64 {
        if !self.is_blocked(source, now) {
            return 0;
        }
        match self.entries.get(&source) {
            Some(window) => window
                .window_start
                .saturating_add(self.window_seconds)
                .saturating_sub(now)
                .max(1),
            None => 0,
        }
    }

    /// Records a failed authentication attempt from `source` at `now`.
    ///
    /// Resets the window when the previous one has expired so that transient
    /// failures do not accumulate forever.
    pub fn record_failure(&mut self, source: IpAddr, now: u64) {
        if self.disabled() {
            return;
        }
        self.prune(now);
        if let Some(window) = self.entries.get_mut(&source) {
            if now.saturating_sub(window.window_start) >= self.window_seconds {
                window.window_start = now;
                window.failures = 0;
            }
            window.failures = window.failures.saturating_add(1);
            return;
        }
        self.insert_bounded(
            source,
            FailureWindow {
                failures: 1,
                window_start: now,
            },
        );
    }

    /// Clears the failure state for `source` after a successful authentication.
    pub fn record_success(&mut self, source: IpAddr) {
        if self.entries.remove(&source).is_some() {
            self.order.retain(|ip| *ip != source);
        }
    }

    /// Inserts a new source, evicting the oldest tracked source (FIFO) when the
    /// capacity is reached so the map stays bounded.
    fn insert_bounded(&mut self, source: IpAddr, window: FailureWindow) {
        while self.entries.len() >= self.max_sources {
            match self.order.pop_front() {
                Some(evicted) => {
                    self.entries.remove(&evicted);
                }
                None => break,
            }
        }
        self.entries.insert(source, window);
        self.order.push_back(source);
    }

    /// Drops sources whose window has fully expired to keep memory bounded.
    fn prune(&mut self, now: u64) {
        if self.entries.is_empty() {
            return;
        }
        let window_seconds = self.window_seconds;
        let entries = &mut self.entries;
        self.order.retain(|ip| {
            let keep = entries
                .get(ip)
                .map(|w| now.saturating_sub(w.window_start) < window_seconds)
                .unwrap_or(false);
            if !keep {
                entries.remove(ip);
            }
            keep
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, n])
    }

    #[test]
    fn blocks_after_budget_exceeded() {
        let mut limiter = AuthRateLimiter::new(3, 60, 16);
        let src = ip(1);
        assert!(!limiter.is_blocked(src, 100));
        limiter.record_failure(src, 100);
        limiter.record_failure(src, 101);
        assert!(!limiter.is_blocked(src, 102));
        limiter.record_failure(src, 102);
        assert!(limiter.is_blocked(src, 103));
        assert!(limiter.retry_after_seconds(src, 103) > 0);
    }

    #[test]
    fn window_resets_after_expiry() {
        let mut limiter = AuthRateLimiter::new(2, 60, 16);
        let src = ip(2);
        limiter.record_failure(src, 100);
        limiter.record_failure(src, 100);
        assert!(limiter.is_blocked(src, 100));
        // Window has elapsed; the source is no longer blocked.
        assert!(!limiter.is_blocked(src, 160));
        // A new failure after expiry starts a fresh window.
        limiter.record_failure(src, 160);
        assert!(!limiter.is_blocked(src, 160));
    }

    #[test]
    fn success_clears_failures() {
        let mut limiter = AuthRateLimiter::new(2, 60, 16);
        let src = ip(3);
        limiter.record_failure(src, 100);
        limiter.record_failure(src, 100);
        assert!(limiter.is_blocked(src, 100));
        limiter.record_success(src);
        assert!(!limiter.is_blocked(src, 100));
    }

    #[test]
    fn per_source_isolation() {
        let mut limiter = AuthRateLimiter::new(1, 60, 16);
        limiter.record_failure(ip(1), 100);
        assert!(limiter.is_blocked(ip(1), 100));
        assert!(!limiter.is_blocked(ip(2), 100));
    }

    #[test]
    fn sources_are_bounded() {
        let mut limiter = AuthRateLimiter::new(5, 600, 2);
        limiter.record_failure(ip(1), 100);
        limiter.record_failure(ip(2), 100);
        limiter.record_failure(ip(3), 100);
        // Only two sources may be tracked; the oldest was evicted.
        assert!(limiter.entries.len() <= 2);
    }

    #[test]
    fn zero_budget_disables_limiter() {
        let mut limiter = AuthRateLimiter::new(0, 60, 16);
        let src = ip(4);
        limiter.record_failure(src, 100);
        limiter.record_failure(src, 100);
        assert!(!limiter.is_blocked(src, 100));
    }
}

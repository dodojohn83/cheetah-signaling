//! Configurable limits for WS-Discovery processing.
//!
//! The core crate is Sans-I/O, so it only exposes limit policies and helpers;
//! the driver enforces them on inbound datagrams.

use crate::error::{OnvifError, OnvifResult};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

/// Size and rate limits for discovery traffic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryLimits {
    /// Maximum XML body size in bytes accepted for parsing.
    pub max_datagram_bytes: usize,
    /// Maximum XML element depth.
    pub max_xml_depth: usize,
    /// Maximum number of XML elements to visit while parsing one message.
    pub max_xml_nodes: usize,
    /// Maximum number of matched devices returned from a single `ProbeMatches`.
    pub max_matches: usize,
    /// Per-source rate limit configuration.
    pub rate: RateLimitConfig,
}

/// Per-source rate limit settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimitConfig {
    /// Sliding window in seconds.
    pub window_seconds: u64,
    /// Maximum datagrams per source IP within the window.
    pub max_per_source: u32,
    /// Maximum distinct source IPs tracked at once.
    pub max_sources: usize,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            max_datagram_bytes: 65_536,
            max_xml_depth: 64,
            max_xml_nodes: 4_096,
            max_matches: 256,
            rate: RateLimitConfig::default(),
        }
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            window_seconds: 60,
            max_per_source: 120,
            max_sources: 1_024,
        }
    }
}

/// A simple per-source token-bucket-like rate limiter for discovery sources.
#[derive(Clone, Debug)]
pub struct DiscoveryRateLimiter {
    config: RateLimitConfig,
    buckets: HashMap<IpAddr, VecDeque<u64>>,
}

impl DiscoveryRateLimiter {
    /// Creates a rate limiter with the supplied configuration.
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: HashMap::with_capacity(config.max_sources),
        }
    }

    /// Returns `true` if the source is allowed to send a datagram at `now`.
    ///
    /// `now` is a monotonic or wall-clock second timestamp supplied by the driver.
    pub fn check(&mut self, source: IpAddr, now: u64) -> bool {
        let cutoff = now.saturating_sub(self.config.window_seconds);
        let max_per_source = self.config.max_per_source as usize;

        // Prune expired entries and remove buckets that became empty so the
        // `max_sources` capacity reflects currently active sources.
        self.buckets.retain(|_, bucket| {
            while bucket.front().is_some_and(|&t| t < cutoff) {
                bucket.pop_front();
            }
            !bucket.is_empty()
        });

        if let Some(bucket) = self.buckets.get_mut(&source) {
            if bucket.len() >= max_per_source {
                return false;
            }
            bucket.push_back(now);
            true
        } else {
            if self.buckets.len() >= self.config.max_sources {
                return false;
            }
            let mut bucket = VecDeque::with_capacity(max_per_source);
            bucket.push_back(now);
            self.buckets.insert(source, bucket);
            true
        }
    }

    /// Removes all source state.
    pub fn reset(&mut self) {
        self.buckets.clear();
    }
}

impl Default for DiscoveryRateLimiter {
    fn default() -> Self {
        Self::new(RateLimitConfig::default())
    }
}

/// Tracks depth and node count while streaming an XML document.
#[derive(Clone, Debug)]
pub struct LimitTracker {
    depth: usize,
    nodes: usize,
    limits: DiscoveryLimits,
}

impl LimitTracker {
    /// Creates a tracker for the supplied limits.
    pub fn new(limits: DiscoveryLimits) -> Self {
        Self {
            depth: 0,
            nodes: 0,
            limits,
        }
    }

    /// Records the start of an element. Returns an error if a limit is exceeded.
    pub fn start(&mut self) -> OnvifResult<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_xml_nodes {
            return Err(OnvifError::LimitExceeded("max xml nodes".to_string()));
        }
        self.depth = self.depth.saturating_add(1);
        if self.depth > self.limits.max_xml_depth {
            return Err(OnvifError::LimitExceeded("max xml depth".to_string()));
        }
        Ok(())
    }

    /// Records an empty element.
    pub fn empty(&mut self) -> OnvifResult<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_xml_nodes {
            return Err(OnvifError::LimitExceeded("max xml nodes".to_string()));
        }
        Ok(())
    }

    /// Records the end of an element.
    pub fn end(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

/// Returns an error if `body.len()` exceeds the configured maximum.
pub fn check_datagram_size(body: &str, limits: &DiscoveryLimits) -> OnvifResult<()> {
    if body.len() > limits.max_datagram_bytes {
        return Err(OnvifError::LimitExceeded(
            "discovery datagram too large".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn rate_limiter_allows_within_limit() {
        let mut rl = DiscoveryRateLimiter::new(RateLimitConfig {
            window_seconds: 10,
            max_per_source: 2,
            max_sources: 10,
        });
        let ip = "192.168.1.1".parse().unwrap();
        assert!(rl.check(ip, 0));
        assert!(rl.check(ip, 1));
        assert!(!rl.check(ip, 2));
    }

    #[test]
    fn rate_limiter_respects_source_capacity() {
        let mut rl = DiscoveryRateLimiter::new(RateLimitConfig {
            window_seconds: 10,
            max_per_source: 1,
            max_sources: 1,
        });
        let a = "192.168.1.1".parse().unwrap();
        let b = "192.168.1.2".parse().unwrap();
        assert!(rl.check(a, 0));
        assert!(!rl.check(b, 0));
    }

    #[test]
    fn rate_limiter_evicts_empty_buckets() {
        let mut rl = DiscoveryRateLimiter::new(RateLimitConfig {
            window_seconds: 10,
            max_per_source: 1,
            max_sources: 1,
        });
        let a = "192.168.1.1".parse().unwrap();
        let b = "192.168.1.2".parse().unwrap();
        assert!(rl.check(a, 0));
        // Advance past the window, so the bucket for `a` becomes empty and removable.
        assert!(rl.check(b, 20));
    }

    #[test]
    fn tracker_rejects_excessive_depth() {
        let limits = DiscoveryLimits {
            max_xml_depth: 1,
            max_xml_nodes: 100,
            ..Default::default()
        };
        let mut t = LimitTracker::new(limits);
        assert!(t.start().is_ok());
        assert!(t.start().is_err());
    }

    #[test]
    fn tracker_rejects_excessive_nodes() {
        let limits = DiscoveryLimits {
            max_xml_depth: 64,
            max_xml_nodes: 1,
            ..Default::default()
        };
        let mut t = LimitTracker::new(limits);
        assert!(t.start().is_ok());
        assert!(t.empty().is_err());
    }

    #[test]
    fn datagram_size_check_rejects_oversized() {
        let limits = DiscoveryLimits {
            max_datagram_bytes: 4,
            ..Default::default()
        };
        assert!(check_datagram_size("hello", &limits).is_err());
        assert!(check_datagram_size("hi", &limits).is_ok());
    }
}

//! Per-source, tenant, protocol and node rate limiter.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Extracts the protocol or resource family from an HTTP path.
///
/// For `/api/v1/<resource>/...` the resource segment is used so that
/// rate limits are scoped per API family instead of all falling under the
/// literal `api` segment. Non-API paths fall back to the first non-empty
/// segment.
pub(crate) fn request_protocol(path: &str) -> String {
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    match parts.next() {
        Some("api") => match parts.next() {
            Some("v1") => parts.next().unwrap_or("api").to_string(),
            Some(second) => second.to_string(),
            None => "api".to_string(),
        },
        Some(first) => first.to_string(),
        None => String::new(),
    }
}

/// Composite key for the token bucket.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RateKey {
    /// Client source IP.
    pub source: IpAddr,
    /// Tenant identifier from the `x-tenant-id` header.
    pub tenant: String,
    /// Protocol or path family, derived from the request path.
    pub protocol: String,
    /// Node identifier handling the request.
    pub node: String,
}

#[derive(Debug)]
struct Bucket {
    last_update: Instant,
    tokens: f64,
}

#[derive(Debug)]
struct Inner {
    buckets: HashMap<RateKey, Bucket>,
    next_cleanup: Instant,
}

/// Simple token-bucket rate limiter with bounded memory.
#[derive(Clone, Debug)]
pub struct RateLimiter {
    capacity: u32,
    refill_per_second: f64,
    stale_after: Duration,
    max_entries: usize,
    inner: Arc<Mutex<Inner>>,
}

impl RateLimiter {
    /// Creates a rate limiter. A zero `capacity` or `refill_per_second`
    /// disables limiting.
    pub fn new(capacity: u32, refill_per_second: u32) -> Self {
        let inner = Inner {
            buckets: HashMap::new(),
            next_cleanup: Instant::now() + Duration::from_secs(60),
        };
        Self {
            capacity,
            refill_per_second: f64::from(refill_per_second),
            stale_after: Duration::from_secs(60),
            max_entries: 10_000,
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Returns true when the key is within its quota.
    pub fn check(&self, key: &RateKey) -> bool {
        if self.capacity == 0 || self.refill_per_second == 0.0 {
            return true;
        }

        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if now >= inner.next_cleanup {
            let deadline = now - self.stale_after;
            inner
                .buckets
                .retain(|_, bucket| bucket.last_update > deadline);
            while inner.buckets.len() > self.max_entries {
                let oldest = inner
                    .buckets
                    .iter()
                    .min_by_key(|(_, bucket)| bucket.last_update)
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    inner.buckets.remove(&k);
                } else {
                    break;
                }
            }
            inner.next_cleanup = now + self.stale_after;
        }

        let bucket = inner.buckets.entry(key.clone()).or_insert(Bucket {
            last_update: now,
            tokens: f64::from(self.capacity),
        });

        let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
        bucket.tokens =
            (bucket.tokens + elapsed * self.refill_per_second).min(f64::from(self.capacity));
        bucket.last_update = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;

/// Axum middleware enforcing per-source, protocol and node rate limits before
/// authentication. A tenant-aware check is also applied in `ApiRequestContext`.
pub async fn rate_limit_middleware(
    State(state): State<Arc<crate::ApiState>>,
    req: Request,
    next: Next,
) -> Response {
    if state.rate_limiter.is_disabled() {
        return next.run(req).await;
    }

    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied()
        .map(|c| c.0.ip())
        .unwrap_or_else(|| [0, 0, 0, 0].into());
    let protocol = request_protocol(req.uri().path());
    let node = state.config.node_id.to_string();
    let key = RateKey {
        source: ip,
        tenant: String::new(),
        protocol,
        node,
    };

    if state.rate_limiter.check(&key) {
        next.run(req).await
    } else {
        crate::HttpError::RateLimited("too many requests".to_string()).into_response()
    }
}

impl RateLimiter {
    /// Returns true when no rate limit is configured.
    pub(crate) fn is_disabled(&self) -> bool {
        self.capacity == 0 || self.refill_per_second == 0.0
    }
}

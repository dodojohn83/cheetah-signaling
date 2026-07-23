//! In-memory capability and service cache for ONVIF device probes.
//!
//! The cache keeps the last known `GetCapabilities` and `GetServices` results per
//! endpoint. Each result has its own freshness timestamp. When a result is
//! younger than the configured TTL it is returned directly. When a refresh
//! fails, the existing result is still returned so the caller keeps a usable
//! (potentially stale) view of the device instead of an empty error.
//!
//! The cache has a fixed capacity; when the limit is reached and a new endpoint
//! is inserted, expired entries are purged first and then the least-recently-known
//! remaining entry is evicted.

use cheetah_onvif_module::{CapabilityKind, CapabilityProbeResult, Service};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A cached view of a device's capabilities and services with independent
/// freshness timestamps.
#[derive(Clone, Debug, Default)]
struct CacheEntry {
    capabilities: Option<HashMap<CapabilityKind, CapabilityProbeResult>>,
    capabilities_fetched_at: Option<Instant>,
    services: Option<Vec<Service>>,
    services_fetched_at: Option<Instant>,
}

impl CacheEntry {
    fn last_seen(&self) -> Option<Instant> {
        match (self.capabilities_fetched_at, self.services_fetched_at) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}

/// Thread-safe capability cache shared between clones of `OnvifHttpDriver`.
#[derive(Clone, Debug)]
pub struct CapabilityCache {
    inner: Arc<Mutex<HashMap<String, CacheEntry>>>,
    capacity: usize,
}

impl CapabilityCache {
    /// Creates an empty cache with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            capacity,
        }
    }

    /// Returns cached capabilities if they exist and have not expired.
    pub fn get_capabilities(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Option<HashMap<CapabilityKind, CapabilityProbeResult>> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key).and_then(|e| {
            e.capabilities_fetched_at.filter(|t| t.elapsed() <= ttl)?;
            e.capabilities.clone()
        })
    }

    /// Returns cached services if they exist and have not expired.
    pub fn get_services(&self, key: &str, ttl: Duration) -> Option<Vec<Service>> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key).and_then(|e| {
            e.services_fetched_at.filter(|t| t.elapsed() <= ttl)?;
            e.services.clone()
        })
    }

    /// Stores a successful capabilities lookup, removing expired entries and
    /// evicting the oldest if the capacity is exceeded.
    pub fn set_capabilities(
        &self,
        key: &str,
        capabilities: HashMap<CapabilityKind, CapabilityProbeResult>,
        ttl: Duration,
    ) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.evict_if_needed(&mut guard, key, ttl);
        let entry = guard.entry(key.to_string()).or_default();
        entry.capabilities = Some(capabilities);
        entry.capabilities_fetched_at = Some(Instant::now());
    }

    /// Stores a successful services lookup, removing expired entries and
    /// evicting the oldest if the capacity is exceeded.
    pub fn set_services(&self, key: &str, services: Vec<Service>, ttl: Duration) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.evict_if_needed(&mut guard, key, ttl);
        let entry = guard.entry(key.to_string()).or_default();
        entry.services = Some(services);
        entry.services_fetched_at = Some(Instant::now());
    }

    /// Returns a stale capabilities entry when a refresh fails, so callers keep
    /// the last usable result even after TTL expiration.
    pub fn stale_capabilities(
        &self,
        key: &str,
    ) -> Option<HashMap<CapabilityKind, CapabilityProbeResult>> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key).and_then(|e| e.capabilities.clone())
    }

    /// Returns stale services when a refresh fails.
    pub fn stale_services(&self, key: &str) -> Option<Vec<Service>> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key).and_then(|e| e.services.clone())
    }

    fn evict_if_needed(&self, guard: &mut HashMap<String, CacheEntry>, key: &str, ttl: Duration) {
        if guard.len() < self.capacity || guard.contains_key(key) {
            return;
        }

        // Capacity reached and the key is new. Expired entries are purged only
        // under pressure so failed refreshes can still fall back to stale data
        // when the cache is below capacity.
        guard.retain(|_, e| e.last_seen().map(|t| t.elapsed() <= ttl).unwrap_or(false));

        if guard.len() < self.capacity {
            return;
        }

        // Still at capacity: evict the entry with the oldest last-seen timestamp.
        let oldest = guard
            .iter()
            .filter_map(|(k, e)| e.last_seen().map(|t| (k.clone(), t)))
            .min_by_key(|(_, t)| *t)
            .map(|(k, _)| k);
        if let Some(k) = oldest {
            guard.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::thread;
    use std::time::Duration;

    fn sample_capabilities() -> HashMap<CapabilityKind, CapabilityProbeResult> {
        let mut m = HashMap::new();
        m.insert(
            CapabilityKind::Device,
            CapabilityProbeResult::Supported {
                namespace: "http://www.onvif.org/ver10/device/wsdl".to_string(),
                xaddr: Some("http://device/onvif/device_service".to_string()),
                version: Some("1.0".to_string()),
            },
        );
        m
    }

    fn sample_services() -> Vec<Service> {
        vec![Service {
            namespace: "http://www.onvif.org/ver10/device/wsdl".to_string(),
            xaddr: "http://device/onvif/device_service".to_string(),
            version: "1.0".to_string(),
        }]
    }

    #[test]
    fn capabilities_and_services_are_cached_and_returned_within_ttl() {
        let cache = CapabilityCache::new(8);
        let caps = sample_capabilities();
        let services = sample_services();

        cache.set_capabilities("k", caps.clone(), Duration::from_secs(60));
        cache.set_services("k", services.clone(), Duration::from_secs(60));

        assert_eq!(
            cache.get_capabilities("k", Duration::from_secs(60)),
            Some(caps)
        );
        assert_eq!(
            cache.get_services("k", Duration::from_secs(60)),
            Some(services)
        );
    }

    #[test]
    fn expired_entries_are_not_returned_but_stale_fallback_keeps_them() {
        let cache = CapabilityCache::new(8);
        let caps = sample_capabilities();
        let services = sample_services();

        cache.set_capabilities("k", caps.clone(), Duration::from_secs(60));
        cache.set_services("k", services.clone(), Duration::from_secs(60));

        // Zero TTL treats every entry as expired.
        assert_eq!(cache.get_capabilities("k", Duration::ZERO), None);
        assert_eq!(cache.get_services("k", Duration::ZERO), None);

        assert_eq!(cache.stale_capabilities("k"), Some(caps));
        assert_eq!(cache.stale_services("k"), Some(services));
    }

    #[test]
    fn capabilities_and_services_have_independent_freshness() {
        let cache = CapabilityCache::new(8);
        let caps = sample_capabilities();
        let services = sample_services();

        cache.set_capabilities("k", caps.clone(), Duration::from_secs(60));

        // Expire capabilities immediately without affecting services.
        assert_eq!(cache.get_capabilities("k", Duration::ZERO), None);

        cache.set_services("k", services.clone(), Duration::from_secs(60));
        assert_eq!(
            cache.get_services("k", Duration::from_secs(60)),
            Some(services)
        );
    }

    #[test]
    fn ttl_expires_entries_after_sleep() {
        let cache = CapabilityCache::new(8);
        let caps = sample_capabilities();

        cache.set_capabilities("k", caps, Duration::from_millis(1));
        thread::sleep(Duration::from_millis(5));
        assert_eq!(cache.get_capabilities("k", Duration::from_millis(1)), None);
    }

    #[test]
    fn capacity_evicts_oldest_entry_when_full() {
        let cache = CapabilityCache::new(2);

        cache.set_capabilities("a", sample_capabilities(), Duration::from_secs(60));
        cache.set_capabilities("b", sample_capabilities(), Duration::from_secs(60));
        cache.set_capabilities("c", sample_capabilities(), Duration::from_secs(60));

        assert_eq!(cache.get_capabilities("a", Duration::from_secs(60)), None);
        assert!(
            cache
                .get_capabilities("b", Duration::from_secs(60))
                .is_some()
        );
        assert!(
            cache
                .get_capabilities("c", Duration::from_secs(60))
                .is_some()
        );
    }

    #[test]
    fn capacity_does_not_exceed_limit_after_many_inserts() {
        let cache = CapabilityCache::new(2);

        for i in 0..10 {
            cache.set_capabilities(
                &format!("k{i}"),
                sample_capabilities(),
                Duration::from_secs(60),
            );
        }

        let mut count = 0;
        for i in 0..10 {
            if cache
                .get_capabilities(&format!("k{i}"), Duration::from_secs(60))
                .is_some()
            {
                count += 1;
            }
        }

        assert_eq!(count, 2);
    }
}

//! Driver configuration.

use crate::util::{MAX_ONVIF_TIMEOUT, clamp_timeout};
use cheetah_onvif_core::discovery::{DiscoveryLimits, RateLimitConfig, XAddrPolicy};
use cheetah_signal_types::DurationMs;
use cheetah_signal_types::config::OnvifConfig;
use std::net::SocketAddr;
use std::time::Duration;

/// Maximum HTTP response body the driver will buffer (64 MiB).
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
/// Maximum WS-Discovery datagram buffer size (2 MiB).
const MAX_DATAGRAM_BYTES: usize = 2 * 1024 * 1024;
/// Maximum concurrent HTTP requests and per-device calls.
const MAX_CONCURRENCY: usize = 1_000_000;
/// Maximum device endpoint permits and cache entries.
const MAX_ENDPOINT_CAPACITY: usize = 1_000_000;
/// Maximum number of matches returned from a single probe round.
const MAX_DISCOVERY_MATCHES: usize = 10_000;
/// Maximum XML depth and node budget for discovery datagrams.
const MAX_XML_DEPTH: usize = 10_000;
const MAX_XML_NODES: usize = 1_000_000;
/// Maximum distinct sources and datagrams per source tracked by the discovery
/// rate limiter.
const MAX_DISCOVERY_SOURCES: usize = 100_000;
const MAX_DISCOVERY_PER_SOURCE: u32 = 100_000;

/// Configuration for the Tokio ONVIF driver.
#[derive(Clone, Debug)]
pub struct DriverConfig {
    /// HTTP connect timeout.
    pub connect_timeout: Duration,
    /// HTTP request timeout (includes body download).
    pub request_timeout: Duration,
    /// Maximum response body size in bytes.
    pub max_response_bytes: usize,
    /// Maximum concurrent HTTP requests per client.
    pub max_concurrent_requests: usize,
    /// Maximum concurrent ONVIF service calls to the same device endpoint.
    pub per_device_concurrency: usize,
    /// Maximum number of device endpoints whose concurrency semaphore is kept
    /// in memory. Idle entries are evicted when the map exceeds this limit.
    pub max_tracked_device_endpoints: usize,
    /// XAddr / stream URI SSRF policy.
    pub xaddr_policy: XAddrPolicy,
    /// WS-Discovery limits.
    pub discovery_limits: DiscoveryLimits,
    /// Multicast group for WS-Discovery Probe (IPv4).
    pub discovery_multicast: SocketAddr,
    /// Local bind address for discovery sockets.
    pub discovery_bind: SocketAddr,
    /// How long to wait for ProbeMatches after sending Probe.
    pub discovery_timeout: DurationMs,
    /// Whether to follow HTTP redirects (each hop re-checked against policy).
    pub follow_redirects: bool,
    /// How long to cache `GetCapabilities`/`GetServices` results per endpoint.
    /// Zero disables caching.
    pub capability_ttl: Duration,
    /// Maximum number of endpoints kept in the capability cache.
    pub capability_cache_capacity: usize,
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
            max_response_bytes: 2 * 1024 * 1024,
            max_concurrent_requests: 32,
            per_device_concurrency: 2,
            max_tracked_device_endpoints: 1_024,
            xaddr_policy: XAddrPolicy::default().with_allow_private(true),
            discovery_limits: DiscoveryLimits::default(),
            // Well-known WS-Discovery multicast endpoint and ephemeral local bind.
            discovery_multicast: SocketAddr::from(([239, 255, 255, 250], 3702)),
            discovery_bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            discovery_timeout: DurationMs::from_millis(3_000),
            follow_redirects: false,
            capability_ttl: Duration::from_secs(300),
            capability_cache_capacity: 1_024,
        }
    }
}

impl From<&OnvifConfig> for DriverConfig {
    fn from(config: &OnvifConfig) -> Self {
        Self {
            connect_timeout: clamp_timeout(Duration::from_millis(
                config.connect_timeout_ms.as_millis().max(0) as u64,
            )),
            request_timeout: clamp_timeout(Duration::from_millis(
                config.request_timeout_ms.as_millis().max(0) as u64,
            )),
            max_response_bytes: config.max_response_bytes.clamp(1, MAX_RESPONSE_BYTES),
            max_concurrent_requests: config.max_concurrent_requests.clamp(1, MAX_CONCURRENCY),
            per_device_concurrency: config.per_device_concurrency.clamp(1, MAX_CONCURRENCY),
            max_tracked_device_endpoints: config
                .max_tracked_device_endpoints
                .clamp(1, MAX_ENDPOINT_CAPACITY),
            xaddr_policy: XAddrPolicy {
                allowed_schemes: config.allowed_schemes.clone(),
                allowed_ports: config.allowed_ports.clone(),
                allow_private: config.allow_private,
                allow_loopback: config.allow_loopback,
                allow_link_local: config.allow_link_local,
                allow_unspecified: config.allow_unspecified,
                allow_domain_names: config.allow_domain_names,
            },
            discovery_limits: DiscoveryLimits {
                max_datagram_bytes: config
                    .discovery_max_datagram_bytes
                    .clamp(1, MAX_DATAGRAM_BYTES),
                max_xml_depth: config.discovery_max_xml_depth.clamp(1, MAX_XML_DEPTH),
                max_xml_nodes: config.discovery_max_xml_nodes.clamp(1, MAX_XML_NODES),
                max_matches: config.discovery_max_matches.clamp(1, MAX_DISCOVERY_MATCHES),
                rate: RateLimitConfig {
                    window_seconds: config.discovery_rate_window_seconds,
                    max_per_source: config
                        .discovery_rate_max_per_source
                        .min(MAX_DISCOVERY_PER_SOURCE),
                    max_sources: config.discovery_rate_max_sources.min(MAX_DISCOVERY_SOURCES),
                },
            },
            discovery_multicast: config.discovery_multicast,
            discovery_bind: config.discovery_bind,
            discovery_timeout: DurationMs::from_millis(
                config
                    .discovery_timeout_ms
                    .as_millis()
                    .clamp(0, MAX_ONVIF_TIMEOUT.as_millis() as i64),
            ),
            follow_redirects: config.follow_redirects,
            capability_ttl: Duration::from_millis(
                config.capability_ttl_ms.as_millis().max(0) as u64
            ),
            capability_cache_capacity: config
                .capability_cache_capacity
                .clamp(1, MAX_ENDPOINT_CAPACITY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::MAX_ONVIF_TIMEOUT;

    #[test]
    fn driver_config_clamps_oversized_limits() {
        let config = OnvifConfig {
            max_response_bytes: usize::MAX,
            max_concurrent_requests: usize::MAX,
            per_device_concurrency: usize::MAX,
            max_tracked_device_endpoints: usize::MAX,
            discovery_max_datagram_bytes: usize::MAX,
            discovery_max_xml_depth: usize::MAX,
            discovery_max_xml_nodes: usize::MAX,
            discovery_max_matches: usize::MAX,
            discovery_rate_max_per_source: u32::MAX,
            discovery_rate_max_sources: usize::MAX,
            capability_cache_capacity: usize::MAX,
            connect_timeout_ms: DurationMs::from_millis(i64::MAX),
            request_timeout_ms: DurationMs::from_millis(i64::MAX),
            discovery_timeout_ms: DurationMs::from_millis(i64::MAX),
            ..Default::default()
        };

        let driver = DriverConfig::from(&config);
        assert_eq!(driver.max_response_bytes, MAX_RESPONSE_BYTES);
        assert_eq!(driver.max_concurrent_requests, MAX_CONCURRENCY);
        assert_eq!(driver.per_device_concurrency, MAX_CONCURRENCY);
        assert_eq!(driver.max_tracked_device_endpoints, MAX_ENDPOINT_CAPACITY);
        assert_eq!(
            driver.discovery_limits.max_datagram_bytes,
            MAX_DATAGRAM_BYTES
        );
        assert_eq!(driver.discovery_limits.max_xml_depth, MAX_XML_DEPTH);
        assert_eq!(driver.discovery_limits.max_xml_nodes, MAX_XML_NODES);
        assert_eq!(driver.discovery_limits.max_matches, MAX_DISCOVERY_MATCHES);
        assert_eq!(
            driver.discovery_limits.rate.max_per_source,
            MAX_DISCOVERY_PER_SOURCE
        );
        assert_eq!(
            driver.discovery_limits.rate.max_sources,
            MAX_DISCOVERY_SOURCES
        );
        assert_eq!(driver.capability_cache_capacity, MAX_ENDPOINT_CAPACITY);
        assert_eq!(driver.connect_timeout, MAX_ONVIF_TIMEOUT);
        assert_eq!(driver.request_timeout, MAX_ONVIF_TIMEOUT);
        assert_eq!(
            driver.discovery_timeout.as_millis(),
            MAX_ONVIF_TIMEOUT.as_millis() as i64
        );
    }
}

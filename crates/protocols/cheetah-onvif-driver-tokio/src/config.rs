//! Driver configuration.

use cheetah_onvif_core::discovery::{DiscoveryLimits, RateLimitConfig, XAddrPolicy};
use cheetah_signal_types::DurationMs;
use cheetah_signal_types::config::OnvifConfig;
use std::net::SocketAddr;
use std::time::Duration;

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
            connect_timeout: Duration::from_millis(
                config.connect_timeout_ms.as_millis().max(0) as u64
            ),
            request_timeout: Duration::from_millis(
                config.request_timeout_ms.as_millis().max(0) as u64
            ),
            max_response_bytes: config.max_response_bytes,
            max_concurrent_requests: config.max_concurrent_requests,
            per_device_concurrency: config.per_device_concurrency.max(1),
            max_tracked_device_endpoints: config.max_tracked_device_endpoints.max(1),
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
                max_datagram_bytes: config.discovery_max_datagram_bytes,
                max_xml_depth: config.discovery_max_xml_depth,
                max_xml_nodes: config.discovery_max_xml_nodes,
                max_matches: config.discovery_max_matches,
                rate: RateLimitConfig {
                    window_seconds: config.discovery_rate_window_seconds,
                    max_per_source: config.discovery_rate_max_per_source,
                    max_sources: config.discovery_rate_max_sources,
                },
            },
            discovery_multicast: config.discovery_multicast,
            discovery_bind: config.discovery_bind,
            discovery_timeout: config.discovery_timeout_ms,
            follow_redirects: config.follow_redirects,
            capability_ttl: Duration::from_millis(
                config.capability_ttl_ms.as_millis().max(0) as u64
            ),
            capability_cache_capacity: config.capability_cache_capacity.max(1),
        }
    }
}

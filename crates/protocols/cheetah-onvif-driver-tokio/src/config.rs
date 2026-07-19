//! Driver configuration.

use cheetah_onvif_core::discovery::{DiscoveryLimits, XAddrPolicy};
use cheetah_signal_types::DurationMs;
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
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
            max_response_bytes: 2 * 1024 * 1024,
            max_concurrent_requests: 32,
            xaddr_policy: XAddrPolicy::default().with_allow_private(true),
            discovery_limits: DiscoveryLimits::default(),
            // Well-known WS-Discovery multicast endpoint and ephemeral local bind.
            discovery_multicast: SocketAddr::from(([239, 255, 255, 250], 3702)),
            discovery_bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            discovery_timeout: DurationMs::from_millis(3_000),
            follow_redirects: false,
        }
    }
}

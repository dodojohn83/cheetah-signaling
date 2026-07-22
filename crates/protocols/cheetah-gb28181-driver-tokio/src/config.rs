//! Driver configuration.

use cheetah_gb28181_core::{CompatibilityProfile, ManagerConfig, SipParserConfig};
use std::net::SocketAddr;
use std::time::Duration;

/// Default maximum UDP datagram size in bytes.
pub const DEFAULT_MAX_DATAGRAM_SIZE: usize = 65535;
/// Default global TCP connection limit.
pub const DEFAULT_MAX_TCP_CONNECTIONS: usize = 1024;
/// Default per-source TCP connection limit.
pub const DEFAULT_MAX_TCP_CONNECTIONS_PER_SOURCE: usize = 16;
/// Default per-read chunk size for TCP streams in bytes.
pub const DEFAULT_TCP_READ_CHUNK_BYTES: usize = 16 * 1024;
/// Default TCP idle timeout.
pub const DEFAULT_TCP_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Default bounded drain deadline applied on shutdown.
pub const DEFAULT_SHUTDOWN_DRAIN: Duration = Duration::from_secs(5);
/// Default access-machine tick interval.
pub const DEFAULT_TICK_INTERVAL: Duration = Duration::from_secs(1);

/// GB28181 transport driver configuration.
///
/// A driver may bind any number of UDP and TCP addresses, mixing IPv4 and IPv6.
/// Connection counts, buffers, timeouts and the shutdown drain deadline are all
/// bounded so the driver never allocates or spawns without an explicit ceiling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriverConfig {
    /// UDP addresses to bind. May be empty when only TCP is used.
    pub udp_binds: Vec<SocketAddr>,
    /// TCP addresses to bind. May be empty when only UDP is used.
    pub tcp_binds: Vec<SocketAddr>,
    /// Maximum size of a single incoming UDP datagram in bytes. Datagrams that
    /// exceed this size are rejected rather than truncated.
    pub max_datagram_size: usize,
    /// Parser limits for incoming SIP messages (UDP and TCP).
    pub parser_config: SipParserConfig,
    /// Global maximum number of simultaneously accepted TCP connections.
    pub max_tcp_connections: usize,
    /// Maximum number of simultaneous TCP connections per source IP address.
    pub max_tcp_connections_per_source: usize,
    /// Per-read chunk size for TCP streams in bytes. The total bytes buffered by
    /// the incremental parser is separately bounded by
    /// [`SipParserConfig::max_buffer_bytes`].
    pub tcp_read_chunk_bytes: usize,
    /// Idle timeout after which an inactive TCP connection is closed.
    pub tcp_idle_timeout: Duration,
    /// Interval between access-machine ticks (timer processing).
    pub tick_interval: Duration,
    /// Bounded deadline for draining in-flight TCP connections on shutdown.
    pub shutdown_drain: Duration,
    /// Bounds (per-role capacity and TTL) and timer configuration for the SIP
    /// transaction tables.
    pub manager_config: ManagerConfig,
    /// Optional compatibility profile applied to incoming and outgoing SIP
    /// parsing/encoding for this listener.
    pub compatibility_profile: Option<CompatibilityProfile>,
}

impl DriverConfig {
    /// Creates a configuration bound to a single UDP address.
    ///
    /// This mirrors the historical single-listener UDP driver. Use the
    /// `with_*` builder methods to add TCP listeners or additional bind
    /// addresses.
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            udp_binds: vec![bind_addr],
            tcp_binds: Vec::new(),
            max_datagram_size: DEFAULT_MAX_DATAGRAM_SIZE,
            parser_config: SipParserConfig::default(),
            max_tcp_connections: DEFAULT_MAX_TCP_CONNECTIONS,
            max_tcp_connections_per_source: DEFAULT_MAX_TCP_CONNECTIONS_PER_SOURCE,
            tcp_read_chunk_bytes: DEFAULT_TCP_READ_CHUNK_BYTES,
            tcp_idle_timeout: DEFAULT_TCP_IDLE_TIMEOUT,
            tick_interval: DEFAULT_TICK_INTERVAL,
            shutdown_drain: DEFAULT_SHUTDOWN_DRAIN,
            manager_config: ManagerConfig::default(),
            compatibility_profile: None,
        }
    }

    /// Creates an empty configuration with no bind addresses.
    pub fn empty() -> Self {
        let mut config = Self::new(SocketAddr::from(([0, 0, 0, 0], 0)));
        config.udp_binds.clear();
        config
    }

    /// Adds a UDP bind address.
    pub fn with_udp_bind(mut self, addr: SocketAddr) -> Self {
        self.udp_binds.push(addr);
        self
    }

    /// Replaces the UDP bind addresses.
    pub fn with_udp_binds(mut self, addrs: impl IntoIterator<Item = SocketAddr>) -> Self {
        self.udp_binds = addrs.into_iter().collect();
        self
    }

    /// Adds a TCP bind address.
    pub fn with_tcp_bind(mut self, addr: SocketAddr) -> Self {
        self.tcp_binds.push(addr);
        self
    }

    /// Replaces the TCP bind addresses.
    pub fn with_tcp_binds(mut self, addrs: impl IntoIterator<Item = SocketAddr>) -> Self {
        self.tcp_binds = addrs.into_iter().collect();
        self
    }

    /// Sets the maximum incoming datagram size.
    pub fn with_max_datagram_size(mut self, size: usize) -> Self {
        self.max_datagram_size = size;
        self
    }

    /// Sets the SIP parser limits.
    pub fn with_parser_config(mut self, config: SipParserConfig) -> Self {
        self.parser_config = config;
        self
    }

    /// Sets the global TCP connection limit.
    pub fn with_max_tcp_connections(mut self, max: usize) -> Self {
        self.max_tcp_connections = max;
        self
    }

    /// Sets the per-source TCP connection limit.
    pub fn with_max_tcp_connections_per_source(mut self, max: usize) -> Self {
        self.max_tcp_connections_per_source = max;
        self
    }

    /// Sets the per-read chunk size for TCP streams.
    pub fn with_tcp_read_chunk_bytes(mut self, bytes: usize) -> Self {
        self.tcp_read_chunk_bytes = bytes;
        self
    }

    /// Sets the TCP idle timeout.
    pub fn with_tcp_idle_timeout(mut self, timeout: Duration) -> Self {
        self.tcp_idle_timeout = timeout;
        self
    }

    /// Sets the access-machine tick interval.
    pub fn with_tick_interval(mut self, interval: Duration) -> Self {
        self.tick_interval = interval;
        self
    }

    /// Sets the bounded shutdown drain deadline.
    pub fn with_shutdown_drain(mut self, drain: Duration) -> Self {
        self.shutdown_drain = drain;
        self
    }

    /// Sets the transaction-table bounds and timer configuration.
    pub fn with_manager_config(mut self, config: ManagerConfig) -> Self {
        self.manager_config = config;
        self
    }

    /// Sets the compatibility profile for SIP parser/encoder normalization.
    pub fn with_compatibility_profile(mut self, profile: Option<CompatibilityProfile>) -> Self {
        self.compatibility_profile = profile;
        self
    }
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self::new(SocketAddr::from(([0, 0, 0, 0], 5060)))
    }
}

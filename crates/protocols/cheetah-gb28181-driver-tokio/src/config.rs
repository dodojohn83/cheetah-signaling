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
/// Default command channel capacity.
pub const DEFAULT_COMMAND_CHANNEL_CAPACITY: usize = 1024;
/// Maximum timeout/deadline duration used by the driver to avoid `tokio::time`
/// `Instant` overflow.
const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
/// Minimum tick interval to avoid `tokio::time::interval` panicking on a zero period.
const MIN_TICK_INTERVAL: Duration = Duration::from_millis(1);
/// Maximum UDP datagram size in bytes. Larger values cannot be represented in a
/// single UDP packet and would allocate an oversized receive buffer.
const MAX_DATAGRAM_SIZE: usize = 65_535;
/// Maximum per-read chunk size for TCP streams. Capping this prevents a
/// misconfigured value from allocating an enormous temporary buffer.
const MAX_TCP_READ_CHUNK_BYTES: usize = 1024 * 1024;
/// Maximum command channel capacity. A zero or huge capacity can break backpressure.
const MAX_COMMAND_CHANNEL_CAPACITY: usize = 65_536;
/// Maximum TCP connections limits. These are per-driver bounds, not global.
const MAX_TCP_CONNECTIONS: usize = 65_536;
const MAX_TCP_CONNECTIONS_PER_SOURCE: usize = 4096;

fn clamp_timeout(d: Duration) -> Duration {
    d.min(MAX_TIMEOUT).max(Duration::from_millis(1))
}

fn clamp_tick_interval(d: Duration) -> Duration {
    d.min(MAX_TIMEOUT).max(MIN_TICK_INTERVAL)
}

fn clamp_max_datagram_size(size: usize) -> usize {
    size.clamp(1, MAX_DATAGRAM_SIZE)
}

fn clamp_tcp_read_chunk_bytes(bytes: usize) -> usize {
    bytes.clamp(1, MAX_TCP_READ_CHUNK_BYTES)
}

fn clamp_command_channel_capacity(capacity: usize) -> usize {
    capacity.clamp(1, MAX_COMMAND_CHANNEL_CAPACITY)
}

fn clamp_max_tcp_connections(max: usize) -> usize {
    max.clamp(1, MAX_TCP_CONNECTIONS)
}

fn clamp_max_tcp_connections_per_source(max: usize) -> usize {
    max.clamp(1, MAX_TCP_CONNECTIONS_PER_SOURCE)
}

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
    /// Bounded capacity of the per-driver command channel.
    pub command_channel_capacity: usize,
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
            command_channel_capacity: DEFAULT_COMMAND_CHANNEL_CAPACITY,
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
        self.max_datagram_size = clamp_max_datagram_size(size);
        self
    }

    /// Sets the SIP parser limits.
    pub fn with_parser_config(mut self, config: SipParserConfig) -> Self {
        self.parser_config = config;
        self
    }

    /// Sets the global TCP connection limit.
    pub fn with_max_tcp_connections(mut self, max: usize) -> Self {
        self.max_tcp_connections = clamp_max_tcp_connections(max);
        self
    }

    /// Sets the per-source TCP connection limit.
    pub fn with_max_tcp_connections_per_source(mut self, max: usize) -> Self {
        self.max_tcp_connections_per_source = clamp_max_tcp_connections_per_source(max);
        self
    }

    /// Sets the per-read chunk size for TCP streams.
    pub fn with_tcp_read_chunk_bytes(mut self, bytes: usize) -> Self {
        self.tcp_read_chunk_bytes = clamp_tcp_read_chunk_bytes(bytes);
        self
    }

    /// Sets the TCP idle timeout.
    pub fn with_tcp_idle_timeout(mut self, timeout: Duration) -> Self {
        self.tcp_idle_timeout = clamp_timeout(timeout);
        self
    }

    /// Sets the access-machine tick interval.
    pub fn with_tick_interval(mut self, interval: Duration) -> Self {
        self.tick_interval = clamp_tick_interval(interval);
        self
    }

    /// Sets the bounded shutdown drain deadline.
    pub fn with_shutdown_drain(mut self, drain: Duration) -> Self {
        self.shutdown_drain = clamp_timeout(drain);
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

    /// Sets the per-driver command channel capacity.
    pub fn with_command_channel_capacity(mut self, capacity: usize) -> Self {
        self.command_channel_capacity = clamp_command_channel_capacity(capacity);
        self
    }
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self::new(SocketAddr::from(([0, 0, 0, 0], 5060)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacities_and_buffer_sizes_are_clamped() {
        let cfg = DriverConfig::new(SocketAddr::from(([0, 0, 0, 0], 5060)))
            .with_max_datagram_size(0)
            .with_tcp_read_chunk_bytes(0)
            .with_command_channel_capacity(0)
            .with_max_tcp_connections(0)
            .with_max_tcp_connections_per_source(0);
        assert_eq!(cfg.max_datagram_size, 1);
        assert_eq!(cfg.tcp_read_chunk_bytes, 1);
        assert_eq!(cfg.command_channel_capacity, 1);
        assert_eq!(cfg.max_tcp_connections, 1);
        assert_eq!(cfg.max_tcp_connections_per_source, 1);

        let cfg = DriverConfig::new(SocketAddr::from(([0, 0, 0, 0], 5060)))
            .with_max_datagram_size(usize::MAX)
            .with_tcp_read_chunk_bytes(usize::MAX)
            .with_command_channel_capacity(usize::MAX)
            .with_max_tcp_connections(usize::MAX)
            .with_max_tcp_connections_per_source(usize::MAX);
        assert_eq!(cfg.max_datagram_size, MAX_DATAGRAM_SIZE);
        assert_eq!(cfg.tcp_read_chunk_bytes, MAX_TCP_READ_CHUNK_BYTES);
        assert_eq!(cfg.command_channel_capacity, MAX_COMMAND_CHANNEL_CAPACITY);
        assert_eq!(cfg.max_tcp_connections, MAX_TCP_CONNECTIONS);
        assert_eq!(
            cfg.max_tcp_connections_per_source,
            MAX_TCP_CONNECTIONS_PER_SOURCE
        );
    }

    #[test]
    fn timeouts_and_tick_interval_are_clamped() {
        let cfg = DriverConfig::new(SocketAddr::from(([0, 0, 0, 0], 5060)))
            .with_tcp_idle_timeout(Duration::MAX)
            .with_shutdown_drain(Duration::MAX)
            .with_tick_interval(Duration::MAX);
        assert_eq!(cfg.tcp_idle_timeout, MAX_TIMEOUT);
        assert_eq!(cfg.shutdown_drain, MAX_TIMEOUT);
        assert_eq!(cfg.tick_interval, MAX_TIMEOUT);

        let cfg = DriverConfig::new(SocketAddr::from(([0, 0, 0, 0], 5060)))
            .with_tcp_idle_timeout(Duration::ZERO)
            .with_shutdown_drain(Duration::ZERO)
            .with_tick_interval(Duration::ZERO);
        assert_eq!(cfg.tcp_idle_timeout, Duration::from_millis(1));
        assert_eq!(cfg.shutdown_drain, Duration::from_millis(1));
        assert_eq!(cfg.tick_interval, MIN_TICK_INTERVAL);
    }
}

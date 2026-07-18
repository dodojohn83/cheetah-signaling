//! Driver configuration.

use cheetah_gb28181_core::SipParserConfig;
use std::net::SocketAddr;

/// UDP driver configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DriverConfig {
    /// Address to bind the UDP socket to.
    pub bind_addr: SocketAddr,
    /// Maximum size of a single incoming UDP datagram in bytes.
    pub max_datagram_size: usize,
    /// Parser limits for incoming SIP messages.
    pub parser_config: SipParserConfig,
}

impl DriverConfig {
    /// Creates a configuration bound to the supplied address.
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            max_datagram_size: 65535,
            parser_config: SipParserConfig::default(),
        }
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
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self::new(SocketAddr::from(([0, 0, 0, 0], 5060)))
    }
}

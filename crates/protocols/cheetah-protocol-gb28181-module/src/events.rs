//! Domain events emitted by the GB28181 access module.

use std::net::SocketAddr;

/// Presence state reported by a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevicePresence {
    /// Device has registered or refreshed registration.
    Online,
    /// Device has explicitly unregistered or expired.
    Offline,
}

/// Events produced by the GB28181 module for downstream consumers.
#[derive(Clone, Debug)]
pub enum Gb28181Event {
    /// A device registered or refreshed registration.
    DeviceRegistered {
        /// Logical domain the device belongs to.
        domain_id: String,
        /// Device identifier from the SIP URI user part.
        device_id: String,
        /// Source address observed from the transport.
        source: SocketAddr,
        /// Parsed Contact endpoint (host:port) for subsequent requests.
        contact: String,
        /// Granted expiry in seconds.
        expires: u32,
        /// Raw User-Agent header, if present.
        user_agent: Option<String>,
    },
    /// A device explicitly unregistered.
    DeviceUnregistered {
        /// Logical domain the device belongs to.
        domain_id: String,
        /// Device identifier from the SIP URI user part.
        device_id: String,
        /// Source address observed from the transport.
        source: SocketAddr,
    },
    /// A keepalive was received.
    Keepalive {
        /// Logical domain the device belongs to.
        domain_id: String,
        /// Device identifier.
        device_id: String,
        /// Source address.
        source: SocketAddr,
        /// Parsed keepalive status.
        status: String,
    },
}

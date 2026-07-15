//! Domain events emitted by the GB28181 access module.

use crate::types::{DeviceId, DomainId};
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
        domain_id: DomainId,
        /// Device identifier from the SIP URI user part.
        device_id: DeviceId,
        /// Source address observed from the transport.
        source: SocketAddr,
        /// Parsed Contact endpoint (host:port) for subsequent requests.
        contact: String,
        /// Granted expiry in seconds.
        expires: u32,
        /// Raw User-Agent header, if present.
        user_agent: Option<String>,
    },
    /// A device explicitly unregistered or its registration expired.
    DeviceUnregistered {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier from the SIP URI user part.
        device_id: DeviceId,
        /// Source address observed from the transport.
        source: SocketAddr,
    },
    /// Device presence changed due to keepalive timeout or recovery.
    DevicePresenceChanged {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// New presence state.
        presence: DevicePresence,
    },
    /// A keepalive was received.
    Keepalive {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Parsed keepalive status.
        status: String,
    },
}

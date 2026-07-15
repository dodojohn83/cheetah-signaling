//! Domain events emitted by the GB28181 access module.

use crate::types::{DeviceId, DomainId};
use crate::xml::{CatalogItem, RecordItem};
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
    /// A catalog response fragment was received.
    CatalogReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Declared total number of items across all fragments.
        sum_num: u32,
        /// Number of items in this fragment.
        num: u32,
        /// Items in this fragment.
        items: Vec<CatalogItem>,
    },
    /// A device info response was received.
    DeviceInfoReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Result string, if present.
        result: Option<String>,
        /// Manufacturer, if present.
        manufacturer: Option<String>,
        /// Model, if present.
        model: Option<String>,
        /// Firmware version, if present.
        firmware: Option<String>,
    },
    /// A device status response was received.
    DeviceStatusReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Result string, if present.
        result: Option<String>,
        /// Online state, if present.
        online: Option<String>,
        /// Status, if present.
        status: Option<String>,
        /// Reason, if present.
        reason: Option<String>,
        /// Invalid equipment flag, if present.
        invalid_equip: Option<String>,
    },
    /// An alarm notification was received.
    AlarmReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Alarm priority.
        priority: Option<String>,
        /// Alarm method.
        method: Option<String>,
        /// Alarm type.
        alarm_type: Option<String>,
        /// Alarm time.
        time: Option<String>,
        /// Extended alarm information.
        info: Option<String>,
    },
    /// A mobile position report was received.
    MobilePositionReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Report time.
        time: Option<String>,
        /// Longitude.
        longitude: Option<String>,
        /// Latitude.
        latitude: Option<String>,
        /// Speed.
        speed: Option<String>,
        /// Direction.
        direction: Option<String>,
        /// Altitude.
        altitude: Option<String>,
    },
    /// A record info response fragment was received.
    RecordInfoReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Device name, if present.
        name: Option<String>,
        /// Declared total number of records across all fragments.
        sum_num: u32,
        /// Number of records in this fragment.
        num: u32,
        /// Records in this fragment.
        items: Vec<RecordItem>,
    },
}

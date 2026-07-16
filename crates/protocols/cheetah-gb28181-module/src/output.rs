//! Outputs produced by the GB28181 protocol module.

use cheetah_gb28181_core::SipMessage;
use cheetah_signal_types::{MessageId, UtcTimestamp};
use std::net::SocketAddr;

/// A protocol-level output emitted by `Gb28181Module`.
#[derive(Clone, Debug)]
pub enum Gb28181Output {
    /// Send a SIP message to the given endpoint.
    SendMessage {
        /// Destination endpoint.
        endpoint: SocketAddr,
        /// SIP message to encode and send.
        message: SipMessage,
    },
    /// Register a new device or refresh an existing registration.
    Register(Gb28181Register),
    /// Device registration refresh with updated endpoint/lease.
    Refresh(Gb28181Refresh),
    /// Device explicitly deregistered.
    Deregister,
    /// Device heartbeat.
    Heartbeat(Gb28181Heartbeat),
    /// Catalog response.
    Catalog(Gb28181Catalog),
    /// Device info response.
    DeviceInfo(Gb28181DeviceInfo),
    /// Device status response.
    DeviceStatus(Gb28181DeviceStatus),
    /// Alarm event.
    Alarm(Gb28181Alarm),
    /// Mobile position report.
    MobilePosition(Gb28181MobilePosition),
    /// Record info response.
    RecordInfo(Gb28181RecordInfo),
    /// Result of a previously submitted command.
    CommandResponse {
        /// Original command id.
        command_id: MessageId,
        /// Serial number used on the wire.
        sn: u32,
        /// Result.
        result: Gb28181CommandResult,
    },
    /// A protocol-level error that should be logged but does not close the session.
    ProtocolError {
        /// Optional source endpoint.
        source: Option<SocketAddr>,
        /// Error kind string for metrics/logging.
        kind: String,
        /// Safe message.
        message: String,
    },
}

/// Device registration request.
#[derive(Clone, Debug)]
pub struct Gb28181Register {
    /// External protocol identity (device ID).
    pub external_id: String,
    /// SIP realm.
    pub realm: String,
    /// Display name, if known.
    pub name: Option<String>,
    /// Device manufacturer.
    pub manufacturer: Option<String>,
    /// Device model.
    pub model: Option<String>,
    /// Firmware version.
    pub firmware: Option<String>,
    /// Remote endpoint used for the registration.
    pub endpoint: SocketAddr,
    /// Registration lease in seconds.
    pub expires_seconds: u32,
    /// Registration timestamp.
    pub registered_at: UtcTimestamp,
}

/// Registration refresh.
#[derive(Clone, Debug)]
pub struct Gb28181Refresh {
    /// External protocol identity.
    pub external_id: String,
    /// Updated endpoint.
    pub endpoint: SocketAddr,
    /// Updated lease.
    pub expires_seconds: u32,
    /// Refresh timestamp.
    pub refreshed_at: UtcTimestamp,
}

/// Heartbeat report.
#[derive(Clone, Debug)]
pub struct Gb28181Heartbeat {
    /// Status string, usually `OK`.
    pub status: String,
    /// Report timestamp.
    pub received_at: UtcTimestamp,
}

/// Catalog response.
#[derive(Clone, Debug)]
pub struct Gb28181Catalog {
    /// Device ID that returned the catalog.
    pub device_id: String,
    /// Serial number.
    pub sn: u32,
    /// Total number of items declared by the device.
    pub sum_num: u32,
    /// Actual items in this fragment.
    pub items: Vec<Gb28181CatalogItem>,
    /// Whether this fragment completes the catalog.
    pub complete: bool,
}

/// A single catalog item.
#[derive(Clone, Debug)]
pub struct Gb28181CatalogItem {
    /// Channel/device identifier.
    pub device_id: String,
    /// Display name.
    pub name: Option<String>,
    /// Channel status.
    pub status: Option<String>,
    /// Parental flag.
    pub parental: Option<u8>,
    /// Parent device/channel ID.
    pub parent_id: Option<String>,
    /// Longitude.
    pub longitude: Option<f64>,
    /// Latitude.
    pub latitude: Option<f64>,
    /// Manufacturer.
    pub manufacturer: Option<String>,
    /// Model.
    pub model: Option<String>,
    /// IP address.
    pub ip_address: Option<String>,
    /// Port.
    pub port: Option<u16>,
}

/// Device info response.
#[derive(Clone, Debug)]
pub struct Gb28181DeviceInfo {
    /// Device or channel identifier.
    pub device_id: String,
    /// Serial number of the response.
    pub sn: u32,
    /// Device name.
    pub name: Option<String>,
    /// Device manufacturer.
    pub manufacturer: Option<String>,
    /// Device model.
    pub model: Option<String>,
    /// Firmware version.
    pub firmware: Option<String>,
    /// Maximum number of cameras declared by the device.
    pub max_camera: Option<u32>,
    /// Maximum number of alarms declared by the device.
    pub max_alarm: Option<u32>,
}

/// Device status response.
#[derive(Clone, Debug)]
pub struct Gb28181DeviceStatus {
    /// Device or channel identifier.
    pub device_id: String,
    /// Serial number of the response.
    pub sn: u32,
    /// Result string, e.g. `OK`.
    pub result: Option<String>,
    /// Online status string.
    pub online: Option<String>,
    /// Overall status string.
    pub status: Option<String>,
    /// Encode status string.
    pub encode: Option<String>,
    /// Record status string.
    pub record: Option<String>,
}

/// Alarm event.
#[derive(Clone, Debug)]
pub struct Gb28181Alarm {
    /// Device or channel identifier.
    pub device_id: String,
    /// Serial number of the alarm report.
    pub sn: u32,
    /// Alarm priority string.
    pub priority: Option<String>,
    /// Alarm method string.
    pub method: Option<String>,
    /// Alarm type string.
    pub alarm_type: Option<String>,
    /// Alarm time string.
    pub alarm_time: Option<String>,
    /// Additional alarm information.
    pub info: Option<String>,
}

/// Record info response.
#[derive(Clone, Debug)]
pub struct Gb28181RecordInfo {
    /// Device or channel identifier.
    pub device_id: String,
    /// Serial number of the response.
    pub sn: u32,
    /// Total number of records declared by the device.
    pub sum_num: u32,
    /// Record items returned so far.
    pub items: Vec<Gb28181RecordItem>,
    /// Whether this fragment completes the record list.
    pub complete: bool,
}

/// A single record item.
#[derive(Clone, Debug)]
pub struct Gb28181RecordItem {
    /// Device or channel identifier.
    pub device_id: String,
    /// Display name.
    pub name: Option<String>,
    /// File path or stream address.
    pub file_path: Option<String>,
    /// Address of the recording.
    pub address: Option<String>,
    /// Start time string.
    pub start_time: Option<String>,
    /// End time string.
    pub end_time: Option<String>,
    /// Secrecy flag.
    pub secrecy: Option<String>,
    /// Record type string.
    pub type_field: Option<String>,
    /// Recorder ID.
    pub recorder_id: Option<String>,
    /// File size in bytes.
    pub file_size: Option<u64>,
}

/// Mobile position report.
#[derive(Clone, Debug)]
pub struct Gb28181MobilePosition {
    /// Device or channel identifier.
    pub device_id: String,
    /// Serial number of the report.
    pub sn: u32,
    /// Position time string.
    pub time: Option<String>,
    /// Longitude string.
    pub longitude: Option<String>,
    /// Latitude string.
    pub latitude: Option<String>,
    /// Speed string.
    pub speed: Option<String>,
    /// Direction string.
    pub direction: Option<String>,
    /// Altitude string.
    pub altitude: Option<String>,
}

/// Result of a command.
#[derive(Clone, Debug)]
pub enum Gb28181CommandResult {
    /// Command accepted/acknowledged.
    Ok,
    /// Device or server error string.
    Error(String),
    /// Command not supported by the device.
    Unsupported,
    /// Timed out waiting for a response.
    Timeout,
}

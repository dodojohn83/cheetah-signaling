//! Data model for GB28181 MANSCDP XML messages.

use serde::Deserialize;

/// Decoded and parsed GB28181 XML message.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Gb28181Message {
    /// Command type, e.g. `Keepalive`, `Catalog`.
    pub cmd_type: String,
    /// Serial number used to correlate requests and responses.
    #[serde(default, alias = "SN")]
    pub sn: Option<String>,
    /// Device or channel identifier.
    #[serde(default, alias = "DeviceID")]
    pub device_id: Option<String>,
    /// Query/command target identifier.
    #[serde(default)]
    pub target_id: Option<String>,
    /// Keepalive / command result status.
    #[serde(default)]
    pub status: Option<String>,
    /// Result code for queries.
    #[serde(default)]
    pub result: Option<String>,
    /// Total number of items when paginating.
    #[serde(default)]
    pub sum_num: Option<u32>,
    /// Catalog item list.
    #[serde(default)]
    pub item_list: Option<ItemList>,
    /// Record item list.
    #[serde(default)]
    pub record_list: Option<RecordList>,
    /// Device name.
    #[serde(default)]
    pub device_name: Option<String>,
    /// Device manufacturer.
    #[serde(default)]
    pub manufacturer: Option<String>,
    /// Device model.
    #[serde(default)]
    pub model: Option<String>,
    /// Firmware version.
    #[serde(default)]
    pub firmware: Option<String>,
    /// Maximum number of cameras declared by the device.
    #[serde(default)]
    pub max_camera: Option<u32>,
    /// Maximum number of alarms declared by the device.
    #[serde(default)]
    pub max_alarm: Option<u32>,
    /// Online status string.
    #[serde(default)]
    pub online: Option<String>,
    /// Encode status string.
    #[serde(default)]
    pub encode: Option<String>,
    /// Record status string.
    #[serde(default)]
    pub record: Option<String>,
    /// Alarm priority string.
    #[serde(default)]
    pub alarm_priority: Option<String>,
    /// Alarm method string.
    #[serde(default)]
    pub alarm_method: Option<String>,
    /// Alarm type string.
    #[serde(default)]
    pub alarm_type: Option<String>,
    /// Alarm time string.
    #[serde(default)]
    pub alarm_time: Option<String>,
    /// Extra information for alarms or commands.
    #[serde(default)]
    pub info: Option<String>,
    /// Position time string.
    #[serde(default)]
    pub time: Option<String>,
    /// Longitude string.
    #[serde(default)]
    pub longitude: Option<String>,
    /// Latitude string.
    #[serde(default)]
    pub latitude: Option<String>,
    /// Speed string.
    #[serde(default)]
    pub speed: Option<String>,
    /// Direction string.
    #[serde(default)]
    pub direction: Option<String>,
    /// Altitude string.
    #[serde(default)]
    pub altitude: Option<String>,
    /// PTZ command hex string for DeviceControl.
    #[serde(default, alias = "PTZCmd")]
    pub ptz_cmd: Option<String>,
    /// Record start time.
    #[serde(default)]
    pub start_time: Option<String>,
    /// Record end time.
    #[serde(default)]
    pub end_time: Option<String>,
    /// File path or stream address.
    #[serde(default, alias = "FilePath")]
    pub file_path: Option<String>,
    /// File size in bytes.
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Root envelope for incoming XML bodies.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Gb28181Envelope {
    /// Device-initiated notification.
    Notify(Gb28181Message),
    /// Response to a server query.
    Response(Gb28181Message),
    /// Server query (used for outgoing requests; may appear in tests).
    Query(Gb28181Message),
    /// Device control request.
    Control(Gb28181Message),
}

impl Gb28181Envelope {
    /// Returns the command type string.
    pub fn cmd_type(&self) -> &str {
        match self {
            Self::Notify(m) | Self::Response(m) | Self::Query(m) | Self::Control(m) => &m.cmd_type,
        }
    }

    /// Returns the inner message.
    pub fn into_message(self) -> Gb28181Message {
        match self {
            Self::Notify(m) | Self::Response(m) | Self::Query(m) | Self::Control(m) => m,
        }
    }
}

/// List container for catalog or record items.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemList {
    /// Items in the list.
    #[serde(default, rename = "Item")]
    pub item: Vec<Item>,
}

/// Catalog item.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    /// Channel/device identifier.
    #[serde(default, alias = "DeviceID")]
    pub device_id: Option<String>,
    /// Human-readable name.
    #[serde(default)]
    pub name: Option<String>,
    /// Channel status.
    #[serde(default)]
    pub status: Option<String>,
    /// Parental flag.
    #[serde(default)]
    pub parental: Option<String>,
    /// Parent device/channel ID.
    #[serde(default, alias = "ParentID")]
    pub parent_id: Option<String>,
    /// Longitude.
    #[serde(default)]
    pub longitude: Option<String>,
    /// Latitude.
    #[serde(default)]
    pub latitude: Option<String>,
    /// Manufacturer.
    #[serde(default)]
    pub manufacturer: Option<String>,
    /// Model.
    #[serde(default)]
    pub model: Option<String>,
    /// Owner.
    #[serde(default)]
    pub owner: Option<String>,
    /// Civil code.
    #[serde(default)]
    pub civil_code: Option<String>,
    /// Address.
    #[serde(default)]
    pub address: Option<String>,
    /// IP address.
    #[serde(default, alias = "IPAddress")]
    pub ip_address: Option<String>,
    /// Port.
    #[serde(default)]
    pub port: Option<u16>,
}

/// Record item list container.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RecordList {
    /// Record items.
    #[serde(default, rename = "Item")]
    pub item: Vec<RecordItem>,
}

/// Record info item.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RecordItem {
    /// Device or channel ID.
    #[serde(default, alias = "DeviceID")]
    pub device_id: Option<String>,
    /// Name.
    #[serde(default)]
    pub name: Option<String>,
    /// File path or stream address.
    #[serde(default, alias = "FilePath")]
    pub file_path: Option<String>,
    /// Address.
    #[serde(default)]
    pub address: Option<String>,
    /// Start time.
    #[serde(default)]
    pub start_time: Option<String>,
    /// End time.
    #[serde(default)]
    pub end_time: Option<String>,
    /// Secrecy flag.
    #[serde(default)]
    pub secrecy: Option<String>,
    /// Type.
    #[serde(default, alias = "Type")]
    pub type_field: Option<String>,
    /// Recorder ID.
    #[serde(default)]
    pub recorder_id: Option<String>,
    /// File size in bytes.
    #[serde(default)]
    pub file_size: Option<u64>,
}

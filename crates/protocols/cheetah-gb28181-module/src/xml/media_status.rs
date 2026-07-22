//! High-level parser for GB28181 `MediaStatus` NOTIFY messages.
//!
//! A `MediaStatus` notification is sent by a device to report a media stream
//! lifecycle change (canonically `NotifyType` `121`, "history playback
//! finished"). The signaling plane only reads the typed `NotifyType` identifier;
//! it never touches RTP/RTCP/PS/TS/ES payloads.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use crate::error::AccessError;

/// Parsed content of a GB28181 `MediaStatus` NOTIFY message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaStatusInfo {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Raw `<NotifyType>` value reported by the device.
    pub notify_type: String,
    /// Unknown extension fields preserved from the message.
    pub extensions: std::collections::HashMap<String, String>,
}

const KNOWN_MEDIA_STATUS_FIELDS: &[&str] = &["CmdType", "SN", "DeviceID", "NotifyType"];

/// Parses a `MediaStatus` message body and returns the extracted fields.
pub fn parse_media_status(body: &[u8]) -> Result<MediaStatusInfo, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_media_status(&root)
}

pub(crate) fn extract_media_status(root: &XmlElement) -> Result<MediaStatusInfo, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "MediaStatus" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(MediaStatusInfo {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        notify_type: root.require_child_text("NotifyType")?,
        extensions: root.extension_map(KNOWN_MEDIA_STATUS_FIELDS),
    })
}

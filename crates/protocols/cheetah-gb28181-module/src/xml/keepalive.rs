//! High-level parser for GB28181 Keepalive messages.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use crate::error::AccessError;

/// Parsed content of a GB28181 `Keepalive` NOTIFY message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeepaliveInfo {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Status string from the `<Status>` element (typically `OK`).
    pub status: String,
    /// Unknown extension fields preserved from the message.
    pub extensions: std::collections::HashMap<String, String>,
}

const KNOWN_KEEPALIVE_FIELDS: &[&str] = &["CmdType", "SN", "DeviceID", "Status"];

/// Parses a `Keepalive` message body and returns the extracted fields.
pub fn parse_keepalive(body: &[u8]) -> Result<KeepaliveInfo, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_keepalive(&root)
}

fn extract_keepalive(root: &XmlElement) -> Result<KeepaliveInfo, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "Keepalive" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(KeepaliveInfo {
        sn: root.child_text("SN").unwrap_or_default(),
        device_id: root.child_text("DeviceID").unwrap_or_default(),
        status: root.child_text("Status").unwrap_or_default(),
        extensions: root.extension_map(KNOWN_KEEPALIVE_FIELDS),
    })
}

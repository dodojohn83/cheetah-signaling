//! High-level parser and encoder for GB28181 Keepalive messages.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use super::writer::encode_xml;
use crate::error::AccessError;
use cheetah_signal_types::clamp_str;
use std::collections::HashMap;

/// Maximum byte length of a single `Keepalive`/`KeepaliveResponse` string field.
const MAX_KEEPALIVE_FIELD_BYTES: usize = 4096;

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

/// Parsed content of a GB28181 `Keepalive` response (`Response`) message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeepaliveResponse {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Result string from the `<Result>` element (typically `OK`).
    pub result: String,
    /// Unknown extension fields preserved from the message.
    pub extensions: std::collections::HashMap<String, String>,
}

const KNOWN_KEEPALIVE_FIELDS: &[&str] = &["CmdType", "SN", "DeviceID", "Status"];
const KNOWN_KEEPALIVE_RESPONSE_FIELDS: &[&str] = &["CmdType", "SN", "DeviceID", "Result"];

/// Parses a `Keepalive` message body and returns the extracted fields.
pub fn parse_keepalive(body: &[u8]) -> Result<KeepaliveInfo, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_keepalive(&root)
}

fn child_element(name: &str, text: &str) -> XmlElement {
    XmlElement {
        name: name.to_string(),
        attributes: HashMap::new(),
        text: text.to_string(),
        children: Vec::new(),
    }
}

/// Encodes a `Keepalive` NOTIFY payload for an upstream platform.
pub fn build_keepalive(sn: &str, device_id: &str, status: &str) -> Result<String, AccessError> {
    let sn = clamp_str(sn, MAX_KEEPALIVE_FIELD_BYTES);
    let device_id = clamp_str(device_id, MAX_KEEPALIVE_FIELD_BYTES);
    let status = clamp_str(status, MAX_KEEPALIVE_FIELD_BYTES);

    let mut root = child_element("Notify", "");
    root.children.push(child_element("CmdType", "Keepalive"));
    root.children.push(child_element("SN", &sn));
    root.children.push(child_element("DeviceID", &device_id));
    root.children.push(child_element("Status", &status));
    encode_xml(&root, true)
}

/// Parses a `Keepalive` response body and returns the extracted fields.
pub fn parse_keepalive_response(body: &[u8]) -> Result<KeepaliveResponse, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_keepalive_response(&root)
}

pub(crate) fn extract_keepalive_response(
    root: &XmlElement,
) -> Result<KeepaliveResponse, AccessError> {
    if root.name != "Response" {
        return Err(AccessError::InvalidXml(format!(
            "expected Response root, got {}",
            root.name
        )));
    }
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "Keepalive" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(KeepaliveResponse {
        sn: clamp_str(&root.require_child_text("SN")?, MAX_KEEPALIVE_FIELD_BYTES),
        device_id: clamp_str(
            &root.require_child_text("DeviceID")?,
            MAX_KEEPALIVE_FIELD_BYTES,
        ),
        result: clamp_str(
            &root.require_child_text("Result")?,
            MAX_KEEPALIVE_FIELD_BYTES,
        ),
        extensions: root.extension_map(KNOWN_KEEPALIVE_RESPONSE_FIELDS),
    })
}

pub(crate) fn extract_keepalive(root: &XmlElement) -> Result<KeepaliveInfo, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "Keepalive" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(KeepaliveInfo {
        sn: clamp_str(&root.require_child_text("SN")?, MAX_KEEPALIVE_FIELD_BYTES),
        device_id: clamp_str(
            &root.require_child_text("DeviceID")?,
            MAX_KEEPALIVE_FIELD_BYTES,
        ),
        status: clamp_str(
            &root.require_child_text("Status")?,
            MAX_KEEPALIVE_FIELD_BYTES,
        ),
        extensions: root.extension_map(KNOWN_KEEPALIVE_FIELDS),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn build_keepalive_clamps_oversized_inputs() {
        let long = "x".repeat(8192);
        let xml = build_keepalive(&long, &long, &long).unwrap();
        assert!(xml.contains("<SN>"));
        assert!(xml.contains("<DeviceID>"));
        assert!(xml.contains("<Status>"));
        assert!(!xml.contains(&"x".repeat(4097)));
    }
}

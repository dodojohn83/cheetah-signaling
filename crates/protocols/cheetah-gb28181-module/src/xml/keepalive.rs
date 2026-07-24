//! High-level parser and encoder for GB28181 Keepalive messages.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use super::writer::encode_xml;
use crate::error::AccessError;
use std::collections::HashMap;

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
    let mut root = child_element("Notify", "");
    root.children.push(child_element("CmdType", "Keepalive"));
    root.children.push(child_element("SN", sn));
    root.children.push(child_element("DeviceID", device_id));
    root.children.push(child_element("Status", status));
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
        return Err(AccessError::invalid_xml(format!(
            "expected Response root, got {}",
            root.name
        )));
    }
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::invalid_xml("missing CmdType"))?;
    if cmd_type != "Keepalive" {
        return Err(AccessError::unsupported_cmd_type(cmd_type));
    }

    Ok(KeepaliveResponse {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        result: root.require_child_text("Result")?,
        extensions: root.extension_map(KNOWN_KEEPALIVE_RESPONSE_FIELDS),
    })
}

pub(crate) fn extract_keepalive(root: &XmlElement) -> Result<KeepaliveInfo, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::invalid_xml("missing CmdType"))?;
    if cmd_type != "Keepalive" {
        return Err(AccessError::unsupported_cmd_type(cmd_type));
    }

    Ok(KeepaliveInfo {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        status: root.require_child_text("Status")?,
        extensions: root.extension_map(KNOWN_KEEPALIVE_FIELDS),
    })
}

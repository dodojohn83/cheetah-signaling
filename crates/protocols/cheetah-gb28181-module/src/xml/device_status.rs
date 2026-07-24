//! GB28181 DeviceStatus response parsing and upstream notify encoding.

use std::collections::HashMap;

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use super::writer::encode_xml;
use crate::error::AccessError;

/// Parsed content of a GB28181 `DeviceStatus` response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceStatusResponse {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Result string (usually `OK`).
    pub result: Option<String>,
    /// Whether the device is online.
    pub online: Option<String>,
    /// Device status reason.
    pub status: Option<String>,
    /// Whether the device is recording.
    pub reason: Option<String>,
    /// Whether the device is invalid.
    pub invalid_equip: Option<String>,
}

/// Parses a `DeviceStatus` response body.
pub fn parse_device_status(body: &[u8]) -> Result<DeviceStatusResponse, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_device_status(&root)
}

fn child_element(name: &str, text: &str) -> XmlElement {
    XmlElement {
        name: name.to_string(),
        attributes: HashMap::new(),
        text: text.to_string(),
        children: Vec::new(),
    }
}

/// Encodes a `DeviceStatus` NOTIFY payload for an upstream platform.
pub fn build_device_status_notify(
    sn: &str,
    device_id: &str,
    online: bool,
) -> Result<String, AccessError> {
    let mut root = child_element("Notify", "");
    root.children.push(child_element("CmdType", "DeviceStatus"));
    root.children.push(child_element("SN", sn));
    root.children.push(child_element("DeviceID", device_id));
    root.children.push(child_element(
        "Online",
        if online { "ONLINE" } else { "OFFLINE" },
    ));
    root.children.push(child_element("Status", "OK"));
    encode_xml(&root, true)
}

pub(crate) fn extract_device_status(
    root: &XmlElement,
) -> Result<DeviceStatusResponse, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::invalid_xml("missing CmdType"))?;
    if cmd_type != "DeviceStatus" {
        return Err(AccessError::unsupported_cmd_type(cmd_type));
    }

    Ok(DeviceStatusResponse {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        result: root.child_text("Result"),
        online: root.child_text("Online"),
        status: root.child_text("Status"),
        reason: root.child_text("Reason"),
        invalid_equip: root.child_text("InvalidEquip"),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_valid_device_status() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>DeviceStatus</CmdType>
    <SN>3</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>OK</Result>
    <Online>ONLINE</Online>
    <Status>OK</Status>
    <Reason></Reason>
    <InvalidEquip>False</InvalidEquip>
</Response>"#;
        let status = parse_device_status(body).unwrap();
        assert_eq!(status.sn, "3");
        assert_eq!(status.device_id, "34020000001320000001");
        assert_eq!(status.result.as_deref(), Some("OK"));
        assert_eq!(status.online.as_deref(), Some("ONLINE"));
        assert_eq!(status.status.as_deref(), Some("OK"));
        assert_eq!(status.invalid_equip.as_deref(), Some("False"));
    }
}

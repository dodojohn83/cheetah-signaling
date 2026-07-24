//! GB28181 DeviceStatus response parsing and upstream notify encoding.

use std::collections::HashMap;

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use super::writer::encode_xml;
use crate::error::AccessError;
use cheetah_signal_types::clamp_str;

/// Maximum byte length of a single `DeviceStatus` string field.
const MAX_DEVICE_STATUS_FIELD_BYTES: usize = 4096;

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

impl DeviceStatusResponse {
    /// Returns a copy with every string field truncated to
    /// [`MAX_DEVICE_STATUS_FIELD_BYTES`] at a UTF-8 boundary.
    pub fn clamp_fields(&self) -> Self {
        fn clamp_opt(s: &Option<String>) -> Option<String> {
            s.as_ref()
                .map(|v| clamp_str(v, MAX_DEVICE_STATUS_FIELD_BYTES))
        }
        Self {
            sn: clamp_str(&self.sn, MAX_DEVICE_STATUS_FIELD_BYTES),
            device_id: clamp_str(&self.device_id, MAX_DEVICE_STATUS_FIELD_BYTES),
            result: clamp_opt(&self.result),
            online: clamp_opt(&self.online),
            status: clamp_opt(&self.status),
            reason: clamp_opt(&self.reason),
            invalid_equip: clamp_opt(&self.invalid_equip),
        }
    }
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
    let sn = clamp_str(sn, MAX_DEVICE_STATUS_FIELD_BYTES);
    let device_id = clamp_str(device_id, MAX_DEVICE_STATUS_FIELD_BYTES);
    let mut root = child_element("Notify", "");
    root.children.push(child_element("CmdType", "DeviceStatus"));
    root.children.push(child_element("SN", &sn));
    root.children.push(child_element("DeviceID", &device_id));
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
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "DeviceStatus" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(DeviceStatusResponse {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        result: root.child_text("Result"),
        online: root.child_text("Online"),
        status: root.child_text("Status"),
        reason: root.child_text("Reason"),
        invalid_equip: root.child_text("InvalidEquip"),
    }
    .clamp_fields())
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

    #[test]
    fn device_status_clamps_oversized_fields() {
        let long = "x".repeat(8192);
        let info = DeviceStatusResponse {
            sn: long.clone(),
            device_id: long.clone(),
            result: Some(long.clone()),
            online: Some(long.clone()),
            status: Some(long.clone()),
            reason: Some(long.clone()),
            invalid_equip: Some(long),
        }
        .clamp_fields();
        assert_eq!(info.sn.len(), 4096);
        assert_eq!(info.device_id.len(), 4096);
        assert_eq!(info.result.as_ref().unwrap().len(), 4096);
        assert_eq!(info.online.as_ref().unwrap().len(), 4096);
        assert_eq!(info.status.as_ref().unwrap().len(), 4096);
        assert_eq!(info.reason.as_ref().unwrap().len(), 4096);
        assert_eq!(info.invalid_equip.as_ref().unwrap().len(), 4096);
    }

    #[test]
    fn clamp_respects_multibyte_utf8_boundary() {
        let long = "é".repeat(4096);
        let info = DeviceStatusResponse {
            sn: long,
            device_id: String::new(),
            result: None,
            online: None,
            status: None,
            reason: None,
            invalid_equip: None,
        }
        .clamp_fields();
        assert!(info.sn.len() <= 4096);
        assert!(info.sn.is_char_boundary(4096));
    }
}

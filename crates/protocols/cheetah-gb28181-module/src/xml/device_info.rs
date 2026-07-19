//! GB28181 DeviceInfo response parsing.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use crate::error::AccessError;

/// Parsed content of a GB28181 `DeviceInfo` response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceInfoResponse {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Result string (usually `OK`).
    pub result: Option<String>,
    /// Manufacturer.
    pub manufacturer: Option<String>,
    /// Model.
    pub model: Option<String>,
    /// Firmware version.
    pub firmware: Option<String>,
    /// Maximum number of channels.
    pub max_camera: Option<String>,
    /// Maximum number of alarms.
    pub max_alarm: Option<String>,
    /// Maximum number of outputs.
    pub max_output: Option<String>,
}

/// Parses a `DeviceInfo` response body.
pub fn parse_device_info(body: &[u8]) -> Result<DeviceInfoResponse, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_device_info(&root)
}

fn extract_device_info(root: &XmlElement) -> Result<DeviceInfoResponse, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "DeviceInfo" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(DeviceInfoResponse {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        result: root.child_text("Result"),
        manufacturer: root.child_text("Manufacturer"),
        model: root.child_text("Model"),
        firmware: root.child_text("Firmware"),
        max_camera: root.child_text("MaxCamera"),
        max_alarm: root.child_text("MaxAlarm"),
        max_output: root.child_text("MaxOutput"),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_valid_device_info() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>DeviceInfo</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>OK</Result>
    <Manufacturer>Hikvision</Manufacturer>
    <Model>DS-2CD</Model>
    <Firmware>V5.5.0</Firmware>
</Response>"#;
        let info = parse_device_info(body).unwrap();
        assert_eq!(info.sn, "1");
        assert_eq!(info.device_id, "34020000001320000001");
        assert_eq!(info.result.as_deref(), Some("OK"));
        assert_eq!(info.manufacturer.as_deref(), Some("Hikvision"));
        assert_eq!(info.model.as_deref(), Some("DS-2CD"));
        assert_eq!(info.firmware.as_deref(), Some("V5.5.0"));
    }
}

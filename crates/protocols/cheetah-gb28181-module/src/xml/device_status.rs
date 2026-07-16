//! GB28181 DeviceStatus response parsing.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
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

fn extract_device_status(root: &XmlElement) -> Result<DeviceStatusResponse, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "DeviceStatus" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(DeviceStatusResponse {
        sn: root.child_text("SN").unwrap_or_default(),
        device_id: root.child_text("DeviceID").unwrap_or_default(),
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

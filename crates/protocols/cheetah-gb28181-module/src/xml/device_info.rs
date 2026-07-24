//! GB28181 DeviceInfo response parsing.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use crate::error::AccessError;
use cheetah_signal_types::clamp_str;

/// Maximum byte length of a single `DeviceInfo` string field.
const MAX_DEVICE_INFO_FIELD_BYTES: usize = 4096;

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

impl DeviceInfoResponse {
    /// Returns a copy with every string field truncated to
    /// [`MAX_DEVICE_INFO_FIELD_BYTES`] at a UTF-8 boundary.
    pub fn clamp_fields(&self) -> Self {
        fn clamp_opt(s: &Option<String>) -> Option<String> {
            s.as_ref()
                .map(|v| clamp_str(v, MAX_DEVICE_INFO_FIELD_BYTES))
        }
        Self {
            sn: clamp_str(&self.sn, MAX_DEVICE_INFO_FIELD_BYTES),
            device_id: clamp_str(&self.device_id, MAX_DEVICE_INFO_FIELD_BYTES),
            result: clamp_opt(&self.result),
            manufacturer: clamp_opt(&self.manufacturer),
            model: clamp_opt(&self.model),
            firmware: clamp_opt(&self.firmware),
            max_camera: clamp_opt(&self.max_camera),
            max_alarm: clamp_opt(&self.max_alarm),
            max_output: clamp_opt(&self.max_output),
        }
    }
}

/// Parses a `DeviceInfo` response body.
pub fn parse_device_info(body: &[u8]) -> Result<DeviceInfoResponse, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_device_info(&root)
}

pub(crate) fn extract_device_info(root: &XmlElement) -> Result<DeviceInfoResponse, AccessError> {
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
    }
    .clamp_fields())
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

    #[test]
    fn device_info_clamps_oversized_fields() {
        let long = "x".repeat(8192);
        let info = DeviceInfoResponse {
            sn: long.clone(),
            device_id: long.clone(),
            result: Some(long.clone()),
            manufacturer: Some(long.clone()),
            model: Some(long.clone()),
            firmware: Some(long.clone()),
            max_camera: Some(long.clone()),
            max_alarm: Some(long.clone()),
            max_output: Some(long.clone()),
        }
        .clamp_fields();
        assert_eq!(info.sn.len(), MAX_DEVICE_INFO_FIELD_BYTES);
        assert_eq!(info.device_id.len(), MAX_DEVICE_INFO_FIELD_BYTES);
        assert_eq!(
            info.manufacturer.as_ref().unwrap().len(),
            MAX_DEVICE_INFO_FIELD_BYTES
        );
        assert!(info.device_id.is_char_boundary(MAX_DEVICE_INFO_FIELD_BYTES));
    }
}

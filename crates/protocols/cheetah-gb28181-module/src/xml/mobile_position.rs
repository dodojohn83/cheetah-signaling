//! GB28181 MobilePosition notification parsing.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use crate::error::AccessError;

/// Parsed content of a GB28181 `MobilePosition` notification.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MobilePositionInfo {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Report time.
    pub time: Option<String>,
    /// Longitude.
    pub longitude: Option<String>,
    /// Latitude.
    pub latitude: Option<String>,
    /// Speed.
    pub speed: Option<String>,
    /// Direction.
    pub direction: Option<String>,
    /// Altitude.
    pub altitude: Option<String>,
}

/// Parses a `MobilePosition` notification body.
pub fn parse_mobile_position(body: &[u8]) -> Result<MobilePositionInfo, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_mobile_position(&root)
}

fn extract_mobile_position(root: &XmlElement) -> Result<MobilePositionInfo, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "MobilePosition" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(MobilePositionInfo {
        sn: root.child_text("SN").unwrap_or_default(),
        device_id: root.child_text("DeviceID").unwrap_or_default(),
        time: root.child_text("Time"),
        longitude: root.child_text("Longitude"),
        latitude: root.child_text("Latitude"),
        speed: root.child_text("Speed"),
        direction: root.child_text("Direction"),
        altitude: root.child_text("Altitude"),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_valid_mobile_position() {
        let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>MobilePosition</CmdType>
    <SN>6</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Time>2026-07-13T14:31:00</Time>
    <Longitude>121.47</Longitude>
    <Latitude>31.23</Latitude>
    <Speed>60.5</Speed>
    <Direction>180</Direction>
    <Altitude>10</Altitude>
</Notify>"#;
        let pos = parse_mobile_position(body).unwrap();
        assert_eq!(pos.sn, "6");
        assert_eq!(pos.device_id, "34020000001320000001");
        assert_eq!(pos.time.as_deref(), Some("2026-07-13T14:31:00"));
        assert_eq!(pos.longitude.as_deref(), Some("121.47"));
        assert_eq!(pos.latitude.as_deref(), Some("31.23"));
        assert_eq!(pos.speed.as_deref(), Some("60.5"));
        assert_eq!(pos.direction.as_deref(), Some("180"));
        assert_eq!(pos.altitude.as_deref(), Some("10"));
    }
}

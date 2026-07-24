//! GB28181 MobilePosition notification parsing and upstream notify encoding.

use std::collections::HashMap;

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use super::writer::encode_xml;
use crate::error::AccessError;
use cheetah_signal_types::clamp_str;

/// Maximum byte length of a single `MobilePositionInfo` string field.
const MAX_MOBILE_POSITION_FIELD_BYTES: usize = 4096;

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

impl MobilePositionInfo {
    /// Returns a copy with every string field truncated to
    /// [`MAX_MOBILE_POSITION_FIELD_BYTES`] at a UTF-8 boundary.
    pub fn clamp_fields(&self) -> Self {
        fn clamp_opt(s: &Option<String>) -> Option<String> {
            s.as_ref()
                .map(|v| clamp_str(v, MAX_MOBILE_POSITION_FIELD_BYTES))
        }
        Self {
            sn: clamp_str(&self.sn, MAX_MOBILE_POSITION_FIELD_BYTES),
            device_id: clamp_str(&self.device_id, MAX_MOBILE_POSITION_FIELD_BYTES),
            time: clamp_opt(&self.time),
            longitude: clamp_opt(&self.longitude),
            latitude: clamp_opt(&self.latitude),
            speed: clamp_opt(&self.speed),
            direction: clamp_opt(&self.direction),
            altitude: clamp_opt(&self.altitude),
        }
    }
}

/// Parses a `MobilePosition` notification body.
pub fn parse_mobile_position(body: &[u8]) -> Result<MobilePositionInfo, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_mobile_position(&root)
}

fn child_element(name: &str, text: &str) -> XmlElement {
    XmlElement {
        name: name.to_string(),
        attributes: HashMap::new(),
        text: text.to_string(),
        children: Vec::new(),
    }
}

/// Encodes a `MobilePosition` NOTIFY payload for an upstream platform.
#[allow(clippy::too_many_arguments)]
pub fn build_mobile_position_notify(
    sn: &str,
    device_id: &str,
    time: Option<&str>,
    longitude: Option<&str>,
    latitude: Option<&str>,
    speed: Option<&str>,
    direction: Option<&str>,
    altitude: Option<&str>,
) -> Result<String, AccessError> {
    let sn = clamp_str(sn, MAX_MOBILE_POSITION_FIELD_BYTES);
    let device_id = clamp_str(device_id, MAX_MOBILE_POSITION_FIELD_BYTES);
    let time = time.map(|t| clamp_str(t, MAX_MOBILE_POSITION_FIELD_BYTES));
    let longitude = longitude.map(|v| clamp_str(v, MAX_MOBILE_POSITION_FIELD_BYTES));
    let latitude = latitude.map(|v| clamp_str(v, MAX_MOBILE_POSITION_FIELD_BYTES));
    let speed = speed.map(|v| clamp_str(v, MAX_MOBILE_POSITION_FIELD_BYTES));
    let direction = direction.map(|v| clamp_str(v, MAX_MOBILE_POSITION_FIELD_BYTES));
    let altitude = altitude.map(|v| clamp_str(v, MAX_MOBILE_POSITION_FIELD_BYTES));

    let mut root = child_element("Notify", "");
    root.children
        .push(child_element("CmdType", "MobilePosition"));
    root.children.push(child_element("SN", &sn));
    root.children.push(child_element("DeviceID", &device_id));
    if let Some(t) = time.as_deref() {
        root.children.push(child_element("Time", t));
    }
    if let Some(v) = longitude.as_deref() {
        root.children.push(child_element("Longitude", v));
    }
    if let Some(v) = latitude.as_deref() {
        root.children.push(child_element("Latitude", v));
    }
    if let Some(v) = speed.as_deref() {
        root.children.push(child_element("Speed", v));
    }
    if let Some(v) = direction.as_deref() {
        root.children.push(child_element("Direction", v));
    }
    if let Some(v) = altitude.as_deref() {
        root.children.push(child_element("Altitude", v));
    }
    encode_xml(&root, true)
}

pub(crate) fn extract_mobile_position(
    root: &XmlElement,
) -> Result<MobilePositionInfo, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "MobilePosition" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(MobilePositionInfo {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        time: root.child_text("Time"),
        longitude: root.child_text("Longitude"),
        latitude: root.child_text("Latitude"),
        speed: root.child_text("Speed"),
        direction: root.child_text("Direction"),
        altitude: root.child_text("Altitude"),
    }
    .clamp_fields())
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

    #[test]
    fn mobile_position_clamps_oversized_fields() {
        let long = "x".repeat(8192);
        let pos = MobilePositionInfo {
            sn: long.clone(),
            device_id: long.clone(),
            time: Some(long.clone()),
            longitude: Some(long.clone()),
            latitude: Some(long.clone()),
            speed: Some(long.clone()),
            direction: Some(long.clone()),
            altitude: Some(long),
        }
        .clamp_fields();
        assert_eq!(pos.sn.len(), 4096);
        assert_eq!(pos.device_id.len(), 4096);
        assert_eq!(pos.time.as_ref().unwrap().len(), 4096);
        assert_eq!(pos.longitude.as_ref().unwrap().len(), 4096);
        assert_eq!(pos.latitude.as_ref().unwrap().len(), 4096);
        assert_eq!(pos.speed.as_ref().unwrap().len(), 4096);
        assert_eq!(pos.direction.as_ref().unwrap().len(), 4096);
        assert_eq!(pos.altitude.as_ref().unwrap().len(), 4096);
    }

    #[test]
    fn build_mobile_position_notify_clamps_oversized_inputs() {
        let long = "x".repeat(8192);
        let xml = build_mobile_position_notify(
            &long,
            &long,
            Some(&long),
            Some(&long),
            Some(&long),
            Some(&long),
            Some(&long),
            Some(&long),
        )
        .unwrap();
        assert!(xml.contains("<SN>"));
        assert!(xml.contains("<DeviceID>"));
        assert!(!xml.contains(&"x".repeat(4097)));
    }
}

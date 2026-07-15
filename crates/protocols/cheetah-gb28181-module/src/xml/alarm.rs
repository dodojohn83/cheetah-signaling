//! GB28181 Alarm notification parsing.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use crate::error::AccessError;

/// Parsed content of a GB28181 `Alarm` notification.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AlarmInfo {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Alarm priority.
    pub priority: Option<String>,
    /// Alarm method.
    pub method: Option<String>,
    /// Alarm type.
    pub alarm_type: Option<String>,
    /// Alarm time.
    pub time: Option<String>,
    /// Extended alarm information.
    pub info: Option<String>,
}

/// Parses an `Alarm` notification body.
pub fn parse_alarm(body: &[u8]) -> Result<AlarmInfo, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_alarm(&root)
}

fn extract_alarm(root: &XmlElement) -> Result<AlarmInfo, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "Alarm" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(AlarmInfo {
        sn: root.child_text("SN").unwrap_or_default(),
        device_id: root.child_text("DeviceID").unwrap_or_default(),
        priority: root.child_text("AlarmPriority"),
        method: root.child_text("AlarmMethod"),
        alarm_type: root.child_text("AlarmType"),
        time: root.child_text("AlarmTime"),
        info: root.child_text("Info"),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_valid_alarm() {
        let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Alarm</CmdType>
    <SN>5</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <AlarmPriority>1</AlarmPriority>
    <AlarmMethod>2</AlarmMethod>
    <AlarmType>1</AlarmType>
    <AlarmTime>2026-07-13T14:31:00</AlarmTime>
    <Info>motion</Info>
</Notify>"#;
        let alarm = parse_alarm(body).unwrap();
        assert_eq!(alarm.sn, "5");
        assert_eq!(alarm.device_id, "34020000001320000001");
        assert_eq!(alarm.priority.as_deref(), Some("1"));
        assert_eq!(alarm.method.as_deref(), Some("2"));
        assert_eq!(alarm.alarm_type.as_deref(), Some("1"));
        assert_eq!(alarm.time.as_deref(), Some("2026-07-13T14:31:00"));
        assert_eq!(alarm.info.as_deref(), Some("motion"));
    }
}

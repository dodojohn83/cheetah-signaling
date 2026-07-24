//! GB28181 Alarm notification parsing and upstream notify encoding.

use std::collections::HashMap;

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use super::writer::encode_xml;
use crate::error::AccessError;
use cheetah_signal_types::clamp_str;

/// Maximum byte length of a single `AlarmInfo` string field.
const MAX_ALARM_FIELD_BYTES: usize = 4096;

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

impl AlarmInfo {
    /// Returns a copy with every string field truncated to
    /// [`MAX_ALARM_FIELD_BYTES`] at a UTF-8 boundary.
    pub fn clamp_fields(&self) -> Self {
        fn clamp_opt(s: &Option<String>) -> Option<String> {
            s.as_ref().map(|v| clamp_str(v, MAX_ALARM_FIELD_BYTES))
        }
        Self {
            sn: clamp_str(&self.sn, MAX_ALARM_FIELD_BYTES),
            device_id: clamp_str(&self.device_id, MAX_ALARM_FIELD_BYTES),
            priority: clamp_opt(&self.priority),
            method: clamp_opt(&self.method),
            alarm_type: clamp_opt(&self.alarm_type),
            time: clamp_opt(&self.time),
            info: clamp_opt(&self.info),
        }
    }
}

/// Parses an `Alarm` notification body.
pub fn parse_alarm(body: &[u8]) -> Result<AlarmInfo, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_alarm(&root)
}

fn child_element(name: &str, text: &str) -> XmlElement {
    XmlElement {
        name: name.to_string(),
        attributes: HashMap::new(),
        text: text.to_string(),
        children: Vec::new(),
    }
}

/// Encodes an `Alarm` NOTIFY payload for an upstream platform.
pub fn build_alarm_notify(
    sn: &str,
    device_id: &str,
    priority: Option<&str>,
    method: Option<&str>,
    alarm_type: Option<&str>,
    time: Option<&str>,
    info: Option<&str>,
) -> Result<String, AccessError> {
    let sn = clamp_str(sn, MAX_ALARM_FIELD_BYTES);
    let device_id = clamp_str(device_id, MAX_ALARM_FIELD_BYTES);
    let priority = priority.map(|p| clamp_str(p, MAX_ALARM_FIELD_BYTES));
    let method = method.map(|m| clamp_str(m, MAX_ALARM_FIELD_BYTES));
    let alarm_type = alarm_type.map(|t| clamp_str(t, MAX_ALARM_FIELD_BYTES));
    let time = time.map(|t| clamp_str(t, MAX_ALARM_FIELD_BYTES));
    let info = info.map(|i| clamp_str(i, MAX_ALARM_FIELD_BYTES));

    let mut root = child_element("Notify", "");
    root.children.push(child_element("CmdType", "Alarm"));
    root.children.push(child_element("SN", &sn));
    root.children.push(child_element("DeviceID", &device_id));
    if let Some(p) = priority.as_deref() {
        root.children.push(child_element("AlarmPriority", p));
    }
    if let Some(m) = method.as_deref() {
        root.children.push(child_element("AlarmMethod", m));
    }
    if let Some(t) = alarm_type.as_deref() {
        root.children.push(child_element("AlarmType", t));
    }
    if let Some(t) = time.as_deref() {
        root.children.push(child_element("AlarmTime", t));
    }
    if let Some(i) = info.as_deref() {
        root.children.push(child_element("Info", i));
    }
    encode_xml(&root, true)
}

pub(crate) fn extract_alarm(root: &XmlElement) -> Result<AlarmInfo, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "Alarm" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(AlarmInfo {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        priority: root.child_text("AlarmPriority"),
        method: root.child_text("AlarmMethod"),
        alarm_type: root.child_text("AlarmType"),
        time: root.child_text("AlarmTime"),
        info: root.child_text("Info"),
    }
    .clamp_fields())
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

    #[test]
    fn alarm_clamps_oversized_fields() {
        let long = "x".repeat(8192);
        let alarm = AlarmInfo {
            sn: long.clone(),
            device_id: long.clone(),
            priority: Some(long.clone()),
            method: Some(long.clone()),
            alarm_type: Some(long.clone()),
            time: Some(long.clone()),
            info: Some(long),
        }
        .clamp_fields();
        assert_eq!(alarm.sn.len(), 4096);
        assert_eq!(alarm.device_id.len(), 4096);
        assert_eq!(alarm.priority.as_ref().unwrap().len(), 4096);
        assert_eq!(alarm.method.as_ref().unwrap().len(), 4096);
        assert_eq!(alarm.alarm_type.as_ref().unwrap().len(), 4096);
        assert_eq!(alarm.time.as_ref().unwrap().len(), 4096);
        assert_eq!(alarm.info.as_ref().unwrap().len(), 4096);
    }

    #[test]
    fn build_alarm_notify_clamps_oversized_inputs() {
        let long = "x".repeat(8192);
        let xml = build_alarm_notify(
            &long,
            &long,
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

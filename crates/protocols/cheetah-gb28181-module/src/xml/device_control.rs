//! GB28181 `DeviceControl` request builder and response parser.
//!
//! Covers the first-version `DeviceControl` subset selected by the project: PTZ
//! control and preset commands. The PTZ byte layout follows the common GB28181
//! 8-byte command used by Monibuca and compatible devices:
//!
//! ```text
//! A5 0F 01 <cmd> <h-speed> <v-speed> <zoom-nibble>0 <checksum>
//! ```
//!
//! where the checksum is the low byte of the sum of the first 7 bytes.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use super::writer::encode_xml;
use crate::error::AccessError;

fn hex_encode_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

const PTZ_FIRST: u8 = 0xA5;
const PTZ_SECOND: u8 = 0x0F;
const PTZ_THIRD: u8 = 0x01;

const RIGHT: u8 = 0x01;
const LEFT: u8 = 0x02;
const DOWN: u8 = 0x04;
const UP: u8 = 0x08;
const ZOOM_IN: u8 = 0x10;
const ZOOM_OUT: u8 = 0x20;

/// A GB28181 PTZ control command.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PtzCommand {
    /// Horizontal speed and direction: negative values pan left, positive pan
    /// right, zero stops panning.
    pub pan: i8,
    /// Vertical speed and direction: negative values tilt down, positive tilt
    /// up, zero stops tilting.
    pub tilt: i8,
    /// Zoom speed and direction: negative values zoom out, positive zoom in,
    /// zero stops zooming.
    pub zoom: i8,
    /// Maximum speed used to clamp pan/tilt values.
    pub max_pan_tilt_speed: u8,
    /// Maximum zoom speed (0-15) used to clamp zoom values.
    pub max_zoom_speed: u8,
}

impl PtzCommand {
    /// Creates a stopped PTZ command with sensible default speed limits.
    pub fn new() -> Self {
        Self {
            max_pan_tilt_speed: 63,
            max_zoom_speed: 15,
            ..Default::default()
        }
    }

    /// Encodes the command as an 8-byte hexadecimal `PTZCmd` string.
    pub fn encode(&self) -> String {
        let mut cmd: u8 = 0;
        let h_speed = clamp_speed_i8(self.pan, self.max_pan_tilt_speed);
        let v_speed = clamp_speed_i8(self.tilt, self.max_pan_tilt_speed);
        // Zoom speed is a 4-bit nibble in the PTZ byte, so it must be capped at 15.
        let zoom_speed = clamp_speed_i8(self.zoom, self.max_zoom_speed.min(15));

        if self.pan > 0 {
            cmd |= RIGHT;
        } else if self.pan < 0 {
            cmd |= LEFT;
        }
        if self.tilt > 0 {
            cmd |= UP;
        } else if self.tilt < 0 {
            cmd |= DOWN;
        }
        if self.zoom > 0 {
            cmd |= ZOOM_IN;
        } else if self.zoom < 0 {
            cmd |= ZOOM_OUT;
        }

        let zoom_byte = zoom_speed << 4;
        let sum = (PTZ_FIRST as u16)
            + (PTZ_SECOND as u16)
            + (PTZ_THIRD as u16)
            + (cmd as u16)
            + (h_speed as u16)
            + (v_speed as u16)
            + (zoom_byte as u16);
        let checksum = (sum & 0xFF) as u8;

        let buf = [
            PTZ_FIRST, PTZ_SECOND, PTZ_THIRD, cmd, h_speed, v_speed, zoom_byte, checksum,
        ];
        hex_encode_upper(&buf)
    }
}

fn text_child(name: &str, text: &str) -> XmlElement {
    XmlElement {
        name: name.to_string(),
        text: text.to_string(),
        ..Default::default()
    }
}

fn clamp_speed_i8(value: i8, max: u8) -> u8 {
    let max_i = max as i16;
    let clamped = value as i16;
    let clamped = clamped.clamp(-max_i, max_i).unsigned_abs();
    clamped as u8
}

/// Preset command actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresetAction {
    /// Set the preset point.
    Set,
    /// Call the preset point.
    Call,
    /// Delete the preset point.
    Delete,
}

impl PresetAction {
    fn cmd_byte(self) -> u8 {
        match self {
            PresetAction::Set => 0x81,
            PresetAction::Call => 0x82,
            PresetAction::Delete => 0x83,
        }
    }
}

/// A preset control command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetCommand {
    /// Preset action to perform.
    pub action: PresetAction,
    /// Preset point number (1-255; 0 may be treated as 1 by devices).
    pub point: u8,
}

impl PresetCommand {
    /// Encodes the preset command as an 8-byte hexadecimal `PTZCmd` string.
    pub fn encode(&self) -> String {
        let mut buf = [0u8; 8];
        buf[0] = PTZ_FIRST;
        buf[1] = PTZ_SECOND;
        buf[2] = 0;
        buf[3] = self.action.cmd_byte();
        buf[4] = 0;
        buf[5] = self.point;
        buf[6] = 0;
        buf[7] = (buf[0..7].iter().map(|b| *b as u16).sum::<u16>() & 0xFF) as u8;
        hex_encode_upper(&buf)
    }
}

/// The subset of `DeviceControl` commands supported in this version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeviceControlKind {
    /// PTZ movement or zoom/focus/iris command.
    Ptz(PtzCommand),
    /// Preset set/call/delete command.
    Preset(PresetCommand),
}

/// A GB28181 `DeviceControl` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceControlRequest {
    /// Sequence number.
    pub sn: String,
    /// Device identifier.
    pub device_id: String,
    /// Control payload.
    pub kind: DeviceControlKind,
}

impl DeviceControlRequest {
    /// Encodes the request as a `DeviceControl` XML body.
    pub fn encode_xml(&self) -> Result<String, AccessError> {
        let ptz_cmd = match &self.kind {
            DeviceControlKind::Ptz(cmd) => cmd.encode(),
            DeviceControlKind::Preset(cmd) => cmd.encode(),
        };

        let control = XmlElement {
            name: "Control".to_string(),
            children: vec![
                text_child("CmdType", "DeviceControl"),
                text_child("SN", &self.sn),
                text_child("DeviceID", &self.device_id),
                text_child("PTZCmd", &ptz_cmd),
            ],
            ..Default::default()
        };

        encode_xml(&control, true)
    }
}

/// A parsed `DeviceControl` response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceControlResponse {
    /// Sequence number.
    pub sn: String,
    /// Device identifier.
    pub device_id: String,
    /// Result reported by the device, if any.
    pub result: Option<String>,
}

/// Parses a `DeviceControl` response body.
pub fn parse_device_control_response(body: &[u8]) -> Result<DeviceControlResponse, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_device_control_response(&root)
}

pub(crate) fn extract_device_control_response(
    root: &XmlElement,
) -> Result<DeviceControlResponse, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "DeviceControl" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    Ok(DeviceControlResponse {
        sn: root.require_child_text("SN")?,
        device_id: root.require_child_text("DeviceID")?,
        result: root.child_text("Result"),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn ptz_right_command_matches_reference_format() {
        let mut cmd = PtzCommand::new();
        cmd.pan = 8;
        cmd.tilt = 0;
        cmd.zoom = 0;
        let encoded = cmd.encode();
        // A5 0F 01 (right=01) h=08 v=00 zoom=00 checksum
        // checksum = (A5+0F+01+01+08+00+00) & FF = BE
        assert_eq!(encoded, "A50F0101080000BE");
    }

    #[test]
    fn ptz_diagonal_with_zoom_command_matches_reference_format() {
        let mut cmd = PtzCommand::new();
        cmd.pan = -8; // left
        cmd.tilt = 8; // up
        cmd.zoom = 5; // zoom in
        let encoded = cmd.encode();
        // cmd = left(2) | up(8) | zoom in(16) = 1A
        // h=08, v=08, zoom byte = 0x50
        // checksum = (A5+0F+01+1A+08+08+50) & FF = 12F & FF = 2F
        assert_eq!(encoded, "A50F011A0808502F");
    }

    #[test]
    fn preset_set_command_has_correct_checksum() {
        let cmd = PresetCommand {
            action: PresetAction::Set,
            point: 1,
        };
        let encoded = cmd.encode();
        assert_eq!(encoded.len(), 16);
        // A5 0F 00 81 00 01 00 checksum
        // checksum = (A5+0F+00+81+00+01+00) & FF = 136 & FF = 36
        assert_eq!(encoded, "A50F008100010036");
    }

    #[test]
    fn device_control_request_xml_contains_ptz_cmd() {
        let mut ptz = PtzCommand::new();
        ptz.pan = 8;
        let req = DeviceControlRequest {
            sn: "42".to_string(),
            device_id: "34020000001320000001".to_string(),
            kind: DeviceControlKind::Ptz(ptz),
        };
        let xml = req.encode_xml().unwrap();
        assert!(xml.contains("<CmdType>DeviceControl</CmdType>"));
        assert!(xml.contains("<SN>42</SN>"));
        assert!(xml.contains("<PTZCmd>A50F0101080000BE</PTZCmd>"));
    }

    #[test]
    fn ptz_zoom_speed_is_capped_at_four_bits() {
        let mut cmd = PtzCommand::new();
        cmd.zoom = 100;
        cmd.max_zoom_speed = 255;
        // Should not panic or overflow; zoom nibble is capped at 15.
        let encoded = cmd.encode();
        // zoom byte = 0xF0, zoom direction bit set, checksum = 0xB5.
        assert_eq!(encoded, "A50F01100000F0B5");
    }

    #[test]
    fn parse_valid_device_control_response() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>DeviceControl</CmdType>
    <SN>42</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>OK</Result>
</Response>"#;
        let resp = parse_device_control_response(body).unwrap();
        assert_eq!(resp.sn, "42");
        assert_eq!(resp.device_id, "34020000001320000001");
        assert_eq!(resp.result.as_deref(), Some("OK"));
    }
}

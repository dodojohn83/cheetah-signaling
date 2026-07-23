//! ONVIF PTZ request builders and response parsers.

use crate::config::ParserLimits;
use crate::error::OnvifModuleError;
use crate::services::parse::{ParseContext, local_name};
use cheetah_onvif_core::soap::Envelope;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;

const PTZ_NS: &str = "http://www.onvif.org/ver20/ptz/wsdl";
const TT_NS: &str = "http://www.onvif.org/ver10/schema";
const CONTINUOUS_MOVE_ACTION: &str = "http://www.onvif.org/ver20/ptz/wsdl/ContinuousMove";
const RELATIVE_MOVE_ACTION: &str = "http://www.onvif.org/ver20/ptz/wsdl/RelativeMove";
const ABSOLUTE_MOVE_ACTION: &str = "http://www.onvif.org/ver20/ptz/wsdl/AbsoluteMove";
const STOP_ACTION: &str = "http://www.onvif.org/ver20/ptz/wsdl/Stop";
const GET_PRESETS_ACTION: &str = "http://www.onvif.org/ver20/ptz/wsdl/GetPresets";
const GOTO_PRESET_ACTION: &str = "http://www.onvif.org/ver20/ptz/wsdl/GotoPreset";

/// Velocity components for continuous move, already clipped by the caller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PtzVelocity {
    /// Pan velocity in device space.
    pub pan: f64,
    /// Tilt velocity in device space.
    pub tilt: f64,
    /// Zoom velocity in device space.
    pub zoom: f64,
}

/// Translation/position components for relative/absolute moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PtzVector {
    /// Pan component.
    pub pan: f64,
    /// Tilt component.
    pub tilt: f64,
    /// Zoom component.
    pub zoom: f64,
}

/// A PTZ preset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtzPreset {
    /// Opaque preset token.
    pub token: String,
    /// Display name.
    pub name: String,
}

fn write_profile_token<W: std::io::Write>(
    writer: &mut Writer<W>,
    token: &str,
) -> Result<(), OnvifModuleError> {
    writer.write_event(Event::Start(BytesStart::new("tptz:ProfileToken")))?;
    writer.write_event(Event::Text(BytesText::new(token)))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:ProfileToken")))?;
    Ok(())
}

fn f64_text(v: f64) -> String {
    // Avoid scientific notation for common PTZ ranges.
    format!("{v:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// Builds a ContinuousMove request.
///
/// Callers must schedule an automatic Stop after a deadline; this builder only
/// produces the wire request.
pub fn continuous_move_request(
    profile_token: &str,
    velocity: PtzVelocity,
    timeout_seconds: Option<u64>,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);

    let mut body = BytesStart::new("tptz:ContinuousMove");
    body.push_attribute(("xmlns:tptz", PTZ_NS));
    body.push_attribute(("xmlns:tt", TT_NS));
    writer.write_event(Event::Start(body))?;
    write_profile_token(&mut writer, profile_token)?;

    writer.write_event(Event::Start(BytesStart::new("tptz:Velocity")))?;
    writer.write_event(Event::Start(BytesStart::new("tptz:PanTilt")))?;
    // Explicit child text fields for broader device compatibility.
    writer.write_event(Event::Start(BytesStart::new("tt:x")))?;
    writer.write_event(Event::Text(BytesText::new(&f64_text(velocity.pan))))?;
    writer.write_event(Event::End(BytesEnd::new("tt:x")))?;
    writer.write_event(Event::Start(BytesStart::new("tt:y")))?;
    writer.write_event(Event::Text(BytesText::new(&f64_text(velocity.tilt))))?;
    writer.write_event(Event::End(BytesEnd::new("tt:y")))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:PanTilt")))?;
    writer.write_event(Event::Start(BytesStart::new("tptz:Zoom")))?;
    writer.write_event(Event::Start(BytesStart::new("tt:x")))?;
    writer.write_event(Event::Text(BytesText::new(&f64_text(velocity.zoom))))?;
    writer.write_event(Event::End(BytesEnd::new("tt:x")))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:Zoom")))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:Velocity")))?;

    if let Some(timeout) = timeout_seconds {
        writer.write_event(Event::Start(BytesStart::new("tptz:Timeout")))?;
        writer.write_event(Event::Text(BytesText::new(&format!("PT{timeout}S"))))?;
        writer.write_event(Event::End(BytesEnd::new("tptz:Timeout")))?;
    }

    writer.write_event(Event::End(BytesEnd::new("tptz:ContinuousMove")))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(CONTINUOUS_MOVE_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Builds a RelativeMove request.
pub fn relative_move_request(
    profile_token: &str,
    translation: PtzVector,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    move_with_vector(
        "RelativeMove",
        RELATIVE_MOVE_ACTION,
        "Translation",
        profile_token,
        translation,
        message_id,
    )
}

/// Builds an AbsoluteMove request.
pub fn absolute_move_request(
    profile_token: &str,
    position: PtzVector,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    move_with_vector(
        "AbsoluteMove",
        ABSOLUTE_MOVE_ACTION,
        "Position",
        profile_token,
        position,
        message_id,
    )
}

fn move_with_vector(
    local: &str,
    action: &str,
    vector_name: &str,
    profile_token: &str,
    vector: PtzVector,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let body_name = format!("tptz:{local}");
    let vector_el = format!("tptz:{vector_name}");

    let mut body = BytesStart::new(&body_name);
    body.push_attribute(("xmlns:tptz", PTZ_NS));
    body.push_attribute(("xmlns:tt", TT_NS));
    writer.write_event(Event::Start(body))?;
    write_profile_token(&mut writer, profile_token)?;
    writer.write_event(Event::Start(BytesStart::new(&vector_el)))?;
    writer.write_event(Event::Start(BytesStart::new("tptz:PanTilt")))?;
    writer.write_event(Event::Start(BytesStart::new("tt:x")))?;
    writer.write_event(Event::Text(BytesText::new(&f64_text(vector.pan))))?;
    writer.write_event(Event::End(BytesEnd::new("tt:x")))?;
    writer.write_event(Event::Start(BytesStart::new("tt:y")))?;
    writer.write_event(Event::Text(BytesText::new(&f64_text(vector.tilt))))?;
    writer.write_event(Event::End(BytesEnd::new("tt:y")))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:PanTilt")))?;
    writer.write_event(Event::Start(BytesStart::new("tptz:Zoom")))?;
    writer.write_event(Event::Start(BytesStart::new("tt:x")))?;
    writer.write_event(Event::Text(BytesText::new(&f64_text(vector.zoom))))?;
    writer.write_event(Event::End(BytesEnd::new("tt:x")))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:Zoom")))?;
    writer.write_event(Event::End(BytesEnd::new(&vector_el)))?;
    writer.write_event(Event::End(BytesEnd::new(&body_name)))?;

    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(action, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Builds a Stop request. Defaults to stopping both pan/tilt and zoom.
pub fn stop_request(
    profile_token: &str,
    pan_tilt: bool,
    zoom: bool,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tptz:Stop");
    body.push_attribute(("xmlns:tptz", PTZ_NS));
    writer.write_event(Event::Start(body))?;
    write_profile_token(&mut writer, profile_token)?;
    writer.write_event(Event::Start(BytesStart::new("tptz:PanTilt")))?;
    writer.write_event(Event::Text(BytesText::new(if pan_tilt {
        "true"
    } else {
        "false"
    })))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:PanTilt")))?;
    writer.write_event(Event::Start(BytesStart::new("tptz:Zoom")))?;
    writer.write_event(Event::Text(BytesText::new(if zoom {
        "true"
    } else {
        "false"
    })))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:Zoom")))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:Stop")))?;

    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(STOP_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Builds a GetPresets request.
pub fn get_presets_request(
    profile_token: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tptz:GetPresets");
    body.push_attribute(("xmlns:tptz", PTZ_NS));
    writer.write_event(Event::Start(body))?;
    write_profile_token(&mut writer, profile_token)?;
    writer.write_event(Event::End(BytesEnd::new("tptz:GetPresets")))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(GET_PRESETS_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Builds a GotoPreset request.
pub fn goto_preset_request(
    profile_token: &str,
    preset_token: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tptz:GotoPreset");
    body.push_attribute(("xmlns:tptz", PTZ_NS));
    writer.write_event(Event::Start(body))?;
    write_profile_token(&mut writer, profile_token)?;
    writer.write_event(Event::Start(BytesStart::new("tptz:PresetToken")))?;
    writer.write_event(Event::Text(BytesText::new(preset_token)))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:PresetToken")))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:GotoPreset")))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(GOTO_PRESET_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Parses GetPresetsResponse into opaque presets.
pub fn parse_get_presets_response(
    xml: &str,
    limits: &ParserLimits,
) -> Result<Vec<PtzPreset>, OnvifModuleError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut presets = Vec::new();
    let mut ctx = ParseContext::new(limits, xml)?;
    let mut current: Option<PtzPreset> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e.name());
                if name == "Preset" {
                    let mut token = String::new();
                    for attr in e.attributes().flatten() {
                        let local = attr.key.local_name();
                        let key = String::from_utf8_lossy(local.as_ref());
                        if key == "token" {
                            token = String::from_utf8_lossy(&attr.value).trim().to_string();
                        }
                    }
                    current = Some(PtzPreset {
                        token,
                        name: String::new(),
                    });
                }
                ctx.on_start(name)?;
            }
            Ok(Event::Empty(_)) => ctx.on_empty()?,
            Ok(Event::Text(e)) => {
                ctx.append_text(&e.xml10_content().unwrap_or_default())?;
            }
            Ok(Event::End(e)) => {
                let parent = ctx.parent().map(str::to_string);
                let name = local_name(&e.name());
                let text = ctx.on_end();
                if name == "Preset" {
                    if let Some(mut preset) = current.take() {
                        if preset.token.is_empty() {
                            return Err(OnvifModuleError::MissingField("Preset/@token".into()));
                        }
                        if preset.name.is_empty() {
                            preset.name = preset.token.clone();
                        }
                        presets.push(preset);
                    }
                } else if name == "Name"
                    && parent.as_deref() == Some("Preset")
                    && let Some(ref mut preset) = current
                {
                    preset.name = text.trim().to_string();
                }
                ctx.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifModuleError::Onvif(
                    cheetah_onvif_core::OnvifError::Xml(e.to_string()),
                ));
            }
            _ => {}
        }
    }

    Ok(presets)
}

/// Clips a velocity component into `[-1.0, 1.0]` (common ONVIF generic space).
pub fn clip_unit(value: f64) -> f64 {
    value.clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn continuous_move_contains_velocity() {
        let xml = continuous_move_request(
            "p1",
            PtzVelocity {
                pan: 0.5,
                tilt: -0.25,
                zoom: 0.0,
            },
            Some(3),
            "urn:uuid:1",
        )
        .unwrap();
        assert!(xml.contains("ContinuousMove"));
        assert!(xml.contains("0.5"));
        assert!(xml.contains("PT3S"));
        assert!(xml.contains("xmlns:tt=\"http://www.onvif.org/ver10/schema\""));
    }

    #[test]
    fn stop_request_contains_flags() {
        let xml = stop_request("p1", true, false, "urn:uuid:1").unwrap();
        assert!(xml.contains("Stop"));
        assert!(xml.contains("false"));
    }

    #[test]
    fn parses_presets() {
        let xml = r#"
        <s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
          <s:Body>
            <tptz:GetPresetsResponse xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl">
              <tptz:Preset token="1">
                <tt:Name xmlns:tt="http://www.onvif.org/ver10/schema">Home</tt:Name>
              </tptz:Preset>
            </tptz:GetPresetsResponse>
          </s:Body>
        </s:Envelope>"#;
        let presets = parse_get_presets_response(xml, &ParserLimits::default()).unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].token, "1");
        assert_eq!(presets[0].name, "Home");
    }
}

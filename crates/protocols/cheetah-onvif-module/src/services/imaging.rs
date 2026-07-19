//! ONVIF Imaging service builders/parsers.
//!
//! v1 is read-only for settings that could irreversibly change exposure or
//! focus. Write helpers return a stable `Unsupported` error so callers do not
//! fabricate success.

use crate::config::ParserLimits;
use crate::error::OnvifModuleError;
use crate::services::parse::{ParseContext, local_name};
use cheetah_onvif_core::soap::Envelope;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;

const IMAGING_NS: &str = "http://www.onvif.org/ver20/imaging/wsdl";
const GET_IMAGING_SETTINGS_ACTION: &str =
    "http://www.onvif.org/ver20/imaging/wsdl/GetImagingSettings";
const GET_OPTIONS_ACTION: &str = "http://www.onvif.org/ver20/imaging/wsdl/GetOptions";
const SET_IMAGING_SETTINGS_ACTION: &str =
    "http://www.onvif.org/ver20/imaging/wsdl/SetImagingSettings";

/// Safe-to-expose imaging diagnostics (read-only).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImagingSettings {
    /// Brightness, if reported.
    pub brightness: Option<f64>,
    /// Color saturation, if reported.
    pub color_saturation: Option<f64>,
    /// Contrast, if reported.
    pub contrast: Option<f64>,
    /// Sharpness, if reported.
    pub sharpness: Option<f64>,
    /// Exposure mode string (e.g. AUTO/MANUAL).
    pub exposure_mode: Option<String>,
    /// Focus mode string.
    pub focus_mode: Option<String>,
}

fn video_source_body(local: &str, video_source_token: &str) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let body_name = format!("timg:{local}");
    let mut body = BytesStart::new(&body_name);
    body.push_attribute(("xmlns:timg", IMAGING_NS));
    writer.write_event(Event::Start(body))?;
    writer.write_event(Event::Start(BytesStart::new("timg:VideoSourceToken")))?;
    writer.write_event(Event::Text(BytesText::new(video_source_token)))?;
    writer.write_event(Event::End(BytesEnd::new("timg:VideoSourceToken")))?;
    writer.write_event(Event::End(BytesEnd::new(&body_name)))?;
    String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))
}

/// Builds GetImagingSettings (read-only).
pub fn get_imaging_settings_request(
    video_source_token: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    Envelope::new(
        GET_IMAGING_SETTINGS_ACTION,
        video_source_body("GetImagingSettings", video_source_token)?,
    )
    .with_message_id(message_id)
    .build()
    .map_err(OnvifModuleError::Onvif)
}

/// Builds GetOptions (read-only capability/options query).
pub fn get_imaging_options_request(
    video_source_token: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    Envelope::new(
        GET_OPTIONS_ACTION,
        video_source_body("GetOptions", video_source_token)?,
    )
    .with_message_id(message_id)
    .build()
    .map_err(OnvifModuleError::Onvif)
}

/// v1 refuses imaging writes that may irreversibly change exposure/focus.
///
/// Returns a stable `Unsupported` error without building a request.
pub fn set_imaging_settings_request(
    _video_source_token: &str,
    _message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    Err(OnvifModuleError::Unsupported(
        "ONVIF SetImagingSettings is not enabled in v1; imaging writes may be irreversible"
            .to_string(),
    ))
}

/// Constant action URL retained for documentation/tests; not used for writes.
pub fn set_imaging_settings_action() -> &'static str {
    SET_IMAGING_SETTINGS_ACTION
}

fn parse_f64(text: &str) -> Option<f64> {
    text.trim().parse().ok()
}

/// Parses GetImagingSettingsResponse into diagnostic fields.
pub fn parse_get_imaging_settings_response(
    xml: &str,
    limits: &ParserLimits,
) -> Result<ImagingSettings, OnvifModuleError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut ctx = ParseContext::new(limits, xml)?;
    let mut settings = ImagingSettings::default();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => ctx.on_start(local_name(&e.name()))?,
            Ok(Event::Empty(_)) => ctx.on_empty()?,
            Ok(Event::Text(e)) => {
                ctx.append_text(&e.xml10_content().unwrap_or_default())?;
            }
            Ok(Event::End(e)) => {
                let name = local_name(&e.name());
                let text = ctx.on_end();
                match name.as_str() {
                    "Brightness" => settings.brightness = parse_f64(&text),
                    "ColorSaturation" => settings.color_saturation = parse_f64(&text),
                    "Contrast" => settings.contrast = parse_f64(&text),
                    "Sharpness" => settings.sharpness = parse_f64(&text),
                    "Mode" => {
                        // Prefer the most specific parent context when present.
                        let parent = ctx.parent().map(str::to_string);
                        match parent.as_deref() {
                            Some("Exposure") => {
                                settings.exposure_mode = Some(text.trim().to_string());
                            }
                            Some("Focus") | Some("AutoFocus") => {
                                settings.focus_mode = Some(text.trim().to_string());
                            }
                            _ => {}
                        }
                    }
                    _ => {}
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

    Ok(settings)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn get_imaging_settings_request_ok() {
        let xml = get_imaging_settings_request("vs1", "urn:uuid:1").unwrap();
        assert!(xml.contains("GetImagingSettings"));
        assert!(xml.contains("vs1"));
    }

    #[test]
    fn set_imaging_settings_is_unsupported() {
        let err = set_imaging_settings_request("vs1", "urn:uuid:1").unwrap_err();
        assert!(matches!(err, OnvifModuleError::Unsupported(_)));
    }

    #[test]
    fn parses_brightness() {
        let xml = r#"
        <s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
          <s:Body>
            <timg:GetImagingSettingsResponse xmlns:timg="http://www.onvif.org/ver20/imaging/wsdl">
              <timg:ImagingSettings>
                <tt:Brightness xmlns:tt="http://www.onvif.org/ver10/schema">50</tt:Brightness>
                <tt:Contrast xmlns:tt="http://www.onvif.org/ver10/schema">40</tt:Contrast>
              </timg:ImagingSettings>
            </timg:GetImagingSettingsResponse>
          </s:Body>
        </s:Envelope>"#;
        let settings = parse_get_imaging_settings_response(xml, &ParserLimits::default()).unwrap();
        assert_eq!(settings.brightness, Some(50.0));
        assert_eq!(settings.contrast, Some(40.0));
    }
}

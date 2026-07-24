//! ONVIF Media / Media2 request builders and response parsers.
//!
//! Media tokens are treated as opaque strings. Stream/Snapshot URIs are returned
//! with credentials still present so the driver/media node can consume them;
//! callers must redact userinfo before logging or northbound responses.

use crate::config::ParserLimits;
use crate::error::OnvifServiceError;
use crate::services::parse::{ParseContext, local_name};
use cheetah_onvif_core::discovery::XAddrPolicy;
use cheetah_onvif_core::soap::Envelope;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;

const MEDIA1_NS: &str = "http://www.onvif.org/ver10/media/wsdl";
const MEDIA2_NS: &str = "http://www.onvif.org/ver20/media/wsdl";

const GET_PROFILES_ACTION_M1: &str = "http://www.onvif.org/ver10/media/wsdl/GetProfiles";
const GET_PROFILES_ACTION_M2: &str = "http://www.onvif.org/ver20/media/wsdl/GetProfiles";
const GET_STREAM_URI_ACTION_M1: &str = "http://www.onvif.org/ver10/media/wsdl/GetStreamUri";
const GET_STREAM_URI_ACTION_M2: &str = "http://www.onvif.org/ver20/media/wsdl/GetStreamUri";
const GET_SNAPSHOT_URI_ACTION_M1: &str = "http://www.onvif.org/ver10/media/wsdl/GetSnapshotUri";
const GET_SNAPSHOT_URI_ACTION_M2: &str = "http://www.onvif.org/ver20/media/wsdl/GetSnapshotUri";

/// Which Media service dialect to target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaDialect {
    /// ONVIF Media 1.0.
    Media1,
    /// ONVIF Media 2.0.
    Media2,
}

impl MediaDialect {
    fn ns(self) -> &'static str {
        match self {
            Self::Media1 => MEDIA1_NS,
            Self::Media2 => MEDIA2_NS,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Media1 => "trt",
            Self::Media2 => "tr2",
        }
    }
}

/// A media profile summary used to map channels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaProfile {
    /// Opaque profile token.
    pub token: String,
    /// Human-readable name when present.
    pub name: String,
    /// Optional fixed flag from the device.
    pub fixed: Option<bool>,
}

/// Stream URI response fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamUri {
    /// Raw URI returned by the device (may include userinfo).
    pub uri: String,
    /// Whether the URI is invalid after connect.
    pub invalid_after_connect: Option<bool>,
    /// Whether the URI is invalid after reboot.
    pub invalid_after_reboot: Option<bool>,
    /// Timeout string if provided (device-specific duration).
    pub timeout: Option<String>,
}

/// Snapshot URI response fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotUri {
    /// Raw URI returned by the device (may include userinfo).
    pub uri: String,
    /// Whether the URI is invalid after connect.
    pub invalid_after_connect: Option<bool>,
    /// Whether the URI is invalid after reboot.
    pub invalid_after_reboot: Option<bool>,
    /// Timeout string if provided.
    pub timeout: Option<String>,
}

fn empty_body(dialect: MediaDialect, local: &str) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let name = format!("{}:{local}", dialect.prefix());
    let xmlns = format!("xmlns:{}", dialect.prefix());
    let mut element = BytesStart::new(&name);
    element.push_attribute((xmlns.as_str(), dialect.ns()));
    writer.write_event(Event::Empty(element))?;
    String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::xml(e)))
}

fn token_body(
    dialect: MediaDialect,
    local: &str,
    token_element: &str,
    token: &str,
) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let body_name = format!("{}:{local}", dialect.prefix());
    let token_name = format!("{}:{token_element}", dialect.prefix());
    let xmlns = format!("xmlns:{}", dialect.prefix());

    let mut body = BytesStart::new(&body_name);
    body.push_attribute((xmlns.as_str(), dialect.ns()));
    writer.write_event(Event::Start(body))?;
    writer.write_event(Event::Start(BytesStart::new(&token_name)))?;
    writer.write_event(Event::Text(BytesText::new(token)))?;
    writer.write_event(Event::End(BytesEnd::new(&token_name)))?;
    writer.write_event(Event::End(BytesEnd::new(&body_name)))?;

    String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::xml(e)))
}

/// Builds a `GetProfiles` request for Media1 or Media2.
pub fn get_profiles_request(
    dialect: MediaDialect,
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    let action = match dialect {
        MediaDialect::Media1 => GET_PROFILES_ACTION_M1,
        MediaDialect::Media2 => GET_PROFILES_ACTION_M2,
    };
    Envelope::new(action, empty_body(dialect, "GetProfiles")?)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Builds a Media1-style `GetStreamUri` request.
pub fn get_stream_uri_request_media1(
    profile_token: &str,
    stream: &str,
    protocol: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);

    let mut body = BytesStart::new("trt:GetStreamUri");
    body.push_attribute(("xmlns:trt", MEDIA1_NS));
    writer.write_event(Event::Start(body))?;

    writer.write_event(Event::Start(BytesStart::new("trt:StreamSetup")))?;
    writer.write_event(Event::Start(BytesStart::new("trt:Stream")))?;
    writer.write_event(Event::Text(BytesText::new(stream)))?;
    writer.write_event(Event::End(BytesEnd::new("trt:Stream")))?;
    writer.write_event(Event::Start(BytesStart::new("trt:Transport")))?;
    writer.write_event(Event::Start(BytesStart::new("trt:Protocol")))?;
    writer.write_event(Event::Text(BytesText::new(protocol)))?;
    writer.write_event(Event::End(BytesEnd::new("trt:Protocol")))?;
    writer.write_event(Event::End(BytesEnd::new("trt:Transport")))?;
    writer.write_event(Event::End(BytesEnd::new("trt:StreamSetup")))?;

    writer.write_event(Event::Start(BytesStart::new("trt:ProfileToken")))?;
    writer.write_event(Event::Text(BytesText::new(profile_token)))?;
    writer.write_event(Event::End(BytesEnd::new("trt:ProfileToken")))?;

    writer.write_event(Event::End(BytesEnd::new("trt:GetStreamUri")))?;

    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::xml(e)))?;
    Envelope::new(GET_STREAM_URI_ACTION_M1, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Builds a Media2 `GetStreamUri` request (protocol + profile token).
pub fn get_stream_uri_request_media2(
    profile_token: &str,
    protocol: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);

    let mut body = BytesStart::new("tr2:GetStreamUri");
    body.push_attribute(("xmlns:tr2", MEDIA2_NS));
    writer.write_event(Event::Start(body))?;
    writer.write_event(Event::Start(BytesStart::new("tr2:Protocol")))?;
    writer.write_event(Event::Text(BytesText::new(protocol)))?;
    writer.write_event(Event::End(BytesEnd::new("tr2:Protocol")))?;
    writer.write_event(Event::Start(BytesStart::new("tr2:ProfileToken")))?;
    writer.write_event(Event::Text(BytesText::new(profile_token)))?;
    writer.write_event(Event::End(BytesEnd::new("tr2:ProfileToken")))?;
    writer.write_event(Event::End(BytesEnd::new("tr2:GetStreamUri")))?;

    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::xml(e)))?;
    Envelope::new(GET_STREAM_URI_ACTION_M2, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Builds a `GetSnapshotUri` request for Media1 or Media2.
pub fn get_snapshot_uri_request(
    dialect: MediaDialect,
    profile_token: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    let action = match dialect {
        MediaDialect::Media1 => GET_SNAPSHOT_URI_ACTION_M1,
        MediaDialect::Media2 => GET_SNAPSHOT_URI_ACTION_M2,
    };
    Envelope::new(
        action,
        token_body(dialect, "GetSnapshotUri", "ProfileToken", profile_token)?,
    )
    .with_message_id(message_id)
    .build()
    .map_err(OnvifServiceError::Onvif)
}

fn parse_bool(text: &str) -> Option<bool> {
    match text.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Parses `GetProfilesResponse` for Media1/Media2.
pub fn parse_get_profiles_response(
    xml: &str,
    limits: &ParserLimits,
) -> Result<Vec<MediaProfile>, OnvifServiceError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut profiles = Vec::new();
    let mut ctx = ParseContext::new(limits, xml)?;
    let mut current: Option<MediaProfile> = None;
    let mut current_token = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e.name());
                if name == "Profiles" || name == "Profile" {
                    current_token.clear();
                    let mut fixed = None;
                    for attr in e.attributes().flatten() {
                        let local = attr.key.local_name();
                        let key = String::from_utf8_lossy(local.as_ref());
                        if key == "token" {
                            current_token = String::from_utf8_lossy(&attr.value).trim().to_string();
                        } else if key == "fixed" {
                            fixed = parse_bool(&String::from_utf8_lossy(&attr.value));
                        }
                    }
                    current = Some(MediaProfile {
                        token: current_token.clone(),
                        name: String::new(),
                        fixed,
                    });
                }
                ctx.on_start(name)?;
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(&e.name());
                if name == "Profiles" || name == "Profile" {
                    let mut token = String::new();
                    for attr in e.attributes().flatten() {
                        let local = attr.key.local_name();
                        let key = String::from_utf8_lossy(local.as_ref());
                        if key == "token" {
                            token = String::from_utf8_lossy(&attr.value).trim().to_string();
                        }
                    }
                    if !token.is_empty() {
                        profiles.push(MediaProfile {
                            token,
                            name: String::new(),
                            fixed: None,
                        });
                    }
                }
                ctx.on_empty()?;
            }
            Ok(Event::Text(e)) => {
                ctx.append_text(&e.xml10_content().unwrap_or_default())?;
            }
            Ok(Event::End(e)) => {
                let parent = ctx.parent().map(str::to_string);
                let name = local_name(&e.name());
                let text = ctx.on_end();
                if (name == "Profiles" || name == "Profile")
                    && let Some(mut profile) = current.take()
                {
                    if profile.token.is_empty() {
                        return Err(OnvifServiceError::missing_field("Profile/@token"));
                    }
                    if profile.name.is_empty() {
                        profile.name = profile.token.clone();
                    }
                    profiles.push(profile);
                } else if let Some(ref mut profile) = current {
                    match name.as_str() {
                        "Name"
                            if matches!(parent.as_deref(), Some("Profiles") | Some("Profile")) =>
                        {
                            profile.name = text.trim().to_string();
                        }
                        "fixed"
                            if matches!(parent.as_deref(), Some("Profiles") | Some("Profile")) =>
                        {
                            profile.fixed = parse_bool(&text);
                        }
                        _ => {}
                    }
                }
                ctx.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifServiceError::Onvif(
                    cheetah_onvif_core::OnvifError::xml(e),
                ));
            }
            _ => {}
        }
    }

    Ok(profiles)
}

fn parse_media_uri(
    xml: &str,
    limits: &ParserLimits,
    uri_parent: &str,
    policy: &XAddrPolicy,
) -> Result<StreamUri, OnvifServiceError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut ctx = ParseContext::new(limits, xml)?;
    let mut uri = StreamUri {
        uri: String::new(),
        invalid_after_connect: None,
        invalid_after_reboot: None,
        timeout: None,
    };

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => ctx.on_start(local_name(&e.name()))?,
            Ok(Event::Empty(_)) => ctx.on_empty()?,
            Ok(Event::Text(e)) => {
                ctx.append_text(&e.xml10_content().unwrap_or_default())?;
            }
            Ok(Event::End(e)) => {
                let parent = ctx.parent().map(str::to_string);
                let name = local_name(&e.name());
                let text = ctx.on_end();
                let in_uri = parent.as_deref() == Some(uri_parent)
                    || parent.as_deref() == Some("MediaUri")
                    || parent.as_deref() == Some("Uri");
                match name.as_str() {
                    "Uri" if in_uri || parent.as_deref() == Some("GetStreamUriResponse") => {
                        uri.uri = text.trim().to_string();
                    }
                    "InvalidAfterConnect" => uri.invalid_after_connect = parse_bool(&text),
                    "InvalidAfterReboot" => uri.invalid_after_reboot = parse_bool(&text),
                    "Timeout" => uri.timeout = Some(text.trim().to_string()),
                    _ => {}
                }
                ctx.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifServiceError::Onvif(
                    cheetah_onvif_core::OnvifError::xml(e),
                ));
            }
            _ => {}
        }
    }

    if uri.uri.is_empty() {
        return Err(OnvifServiceError::missing_field("Uri"));
    }
    validate_media_uri(&uri.uri, policy)?;
    Ok(uri)
}

/// Validates a media stream/snapshot URI against an extended SSRF policy.
///
/// Userinfo is stripped before host checks so device credentials in the URI do
/// not trigger the XAddr userinfo rejection; credentials must still be redacted
/// before logging or northbound emission via [`redact_uri_userinfo`].
pub fn validate_media_uri(uri: &str, base: &XAddrPolicy) -> Result<(), OnvifServiceError> {
    let parsed = url::Url::parse(uri)
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::invalid_xaddr(e)))?;
    let mut policy = base.clone();
    for scheme in ["rtsp", "rtsps", "http", "https"] {
        if !policy.allowed_schemes.iter().any(|s| s == scheme) {
            policy.allowed_schemes.push(scheme.to_string());
        }
    }
    for port in [80_u16, 443, 554, 8554] {
        if !policy.allowed_ports.is_empty() && !policy.allowed_ports.contains(&port) {
            policy.allowed_ports.push(port);
        }
    }
    let mut sanitized = parsed;
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    policy
        .validate(&sanitized)
        .map_err(OnvifServiceError::Onvif)
}

/// Parses `GetStreamUriResponse`.
pub fn parse_get_stream_uri_response(
    xml: &str,
    limits: &ParserLimits,
    policy: &XAddrPolicy,
) -> Result<StreamUri, OnvifServiceError> {
    parse_media_uri(xml, limits, "MediaUri", policy)
}

/// Parses `GetSnapshotUriResponse`.
pub fn parse_get_snapshot_uri_response(
    xml: &str,
    limits: &ParserLimits,
    policy: &XAddrPolicy,
) -> Result<SnapshotUri, OnvifServiceError> {
    let stream = parse_media_uri(xml, limits, "MediaUri", policy)?;
    Ok(SnapshotUri {
        uri: stream.uri,
        invalid_after_connect: stream.invalid_after_connect,
        invalid_after_reboot: stream.invalid_after_reboot,
        timeout: stream.timeout,
    })
}

/// Redacts userinfo from a URI for logs and northbound APIs.
pub fn redact_uri_userinfo(uri: &str) -> String {
    match url::Url::parse(uri) {
        Ok(mut parsed) => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string()
        }
        Err(_) => {
            // Best-effort: strip scheme://user:pass@
            if let Some(idx) = uri.find("://") {
                let (scheme, rest) = uri.split_at(idx + 3);
                if let Some(at) = rest.find('@') {
                    return format!("{scheme}{}", &rest[at + 1..]);
                }
            }
            uri.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use cheetah_onvif_core::discovery::XAddrPolicy;

    #[test]
    fn get_profiles_request_contains_action() {
        let xml = get_profiles_request(MediaDialect::Media2, "urn:uuid:1").unwrap();
        assert!(xml.contains("GetProfiles"));
        assert!(xml.contains(MEDIA2_NS));
    }

    #[test]
    fn parses_profiles_with_token_attribute() {
        let xml = r#"
        <s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
          <s:Body>
            <trt:GetProfilesResponse xmlns:trt="http://www.onvif.org/ver10/media/wsdl">
              <trt:Profiles token="profile1" fixed="true">
                <tt:Name xmlns:tt="http://www.onvif.org/ver10/schema">MainStream</tt:Name>
              </trt:Profiles>
            </trt:GetProfilesResponse>
          </s:Body>
        </s:Envelope>"#;
        let profiles = parse_get_profiles_response(xml, &ParserLimits::default()).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].token, "profile1");
        assert_eq!(profiles[0].name, "MainStream");
        assert_eq!(profiles[0].fixed, Some(true));
    }

    #[test]
    fn parse_stream_uri_validates_policy() {
        let xml = r#"
        <s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
          <s:Body>
            <trt:GetStreamUriResponse xmlns:trt="http://www.onvif.org/ver10/media/wsdl">
              <trt:MediaUri>
                <tt:Uri xmlns:tt="http://www.onvif.org/ver10/schema">rtsp://192.0.2.10:554/stream</tt:Uri>
              </trt:MediaUri>
            </trt:GetStreamUriResponse>
          </s:Body>
        </s:Envelope>"#;
        // TEST-NET address with media ports allowed via validate_media_uri.
        let policy = XAddrPolicy::default().with_allow_private(true);
        let uri = parse_get_stream_uri_response(xml, &ParserLimits::default(), &policy).unwrap();
        assert_eq!(uri.uri, "rtsp://192.0.2.10:554/stream");
    }

    #[test]
    fn redact_userinfo_from_rtsp() {
        let redacted = redact_uri_userinfo("rtsp://user:secret@192.0.2.10/stream");
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("192.0.2.10"));
    }
}

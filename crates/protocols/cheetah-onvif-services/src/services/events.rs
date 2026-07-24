//! ONVIF Events PullPoint request builders and response parsers.

use crate::config::ParserLimits;
use crate::error::OnvifServiceError;
use crate::services::parse::{ParseContext, local_name};
use cheetah_onvif_core::discovery::XAddrPolicy;
use cheetah_onvif_core::soap::Envelope;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;

const EVENTS_NS: &str = "http://www.onvif.org/ver10/events/wsdl";
const WSNT_NS: &str = "http://docs.oasis-open.org/wsn/b-2";
const CREATE_PULLPOINT_ACTION: &str =
    "http://www.onvif.org/ver10/events/wsdl/CreatePullPointSubscription";
const PULL_MESSAGES_ACTION: &str =
    "http://www.onvif.org/ver10/events/wsdl/PullPointSubscription/PullMessages";

/// SOAP action for Renew.
pub const RENEW_ACTION: &str = "http://www.onvif.org/ver10/events/wsdl/PullPointSubscription/Renew";

/// SOAP action for Unsubscribe.
pub const UNSUBSCRIBE_ACTION: &str =
    "http://www.onvif.org/ver10/events/wsdl/PullPointSubscription/Unsubscribe";

/// Result of creating a pull-point subscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PullPointSubscription {
    /// Subscription manager / pull-point endpoint.
    pub subscription_reference: String,
    /// Absolute or relative termination time as returned by the device.
    pub termination_time: Option<String>,
    /// Current time reported by the device, if any.
    pub current_time: Option<String>,
}

/// A normalized ONVIF notification message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnvifNotification {
    /// Topic string (may be hierarchical path).
    pub topic: String,
    /// UTC time string from the device when present.
    pub utc_time: Option<String>,
    /// Property operation (Initialized/Changed/Deleted) when present.
    pub property_operation: Option<String>,
    /// Bounded raw Source/Key/Data fragment for vendor topics.
    pub extension_xml: Option<String>,
}

/// Builds CreatePullPointSubscription.
pub fn create_pull_point_subscription_request(
    initial_termination_time: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tev:CreatePullPointSubscription");
    body.push_attribute(("xmlns:tev", EVENTS_NS));
    writer.write_event(Event::Start(body))?;
    writer.write_event(Event::Start(BytesStart::new("tev:InitialTerminationTime")))?;
    writer.write_event(Event::Text(BytesText::new(initial_termination_time)))?;
    writer.write_event(Event::End(BytesEnd::new("tev:InitialTerminationTime")))?;
    writer.write_event(Event::End(BytesEnd::new("tev:CreatePullPointSubscription")))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::xml(e)))?;
    Envelope::new(CREATE_PULLPOINT_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Builds PullMessages.
pub fn pull_messages_request(
    timeout: &str,
    message_limit: u32,
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tev:PullMessages");
    body.push_attribute(("xmlns:tev", EVENTS_NS));
    writer.write_event(Event::Start(body))?;
    writer.write_event(Event::Start(BytesStart::new("tev:Timeout")))?;
    writer.write_event(Event::Text(BytesText::new(timeout)))?;
    writer.write_event(Event::End(BytesEnd::new("tev:Timeout")))?;
    writer.write_event(Event::Start(BytesStart::new("tev:MessageLimit")))?;
    writer.write_event(Event::Text(BytesText::new(&message_limit.to_string())))?;
    writer.write_event(Event::End(BytesEnd::new("tev:MessageLimit")))?;
    writer.write_event(Event::End(BytesEnd::new("tev:PullMessages")))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::xml(e)))?;
    Envelope::new(PULL_MESSAGES_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Builds Renew with a termination time.
pub fn renew_request(
    termination_time: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tev:Renew");
    body.push_attribute(("xmlns:tev", EVENTS_NS));
    body.push_attribute(("xmlns:wsnt", WSNT_NS));
    writer.write_event(Event::Start(body))?;
    writer.write_event(Event::Start(BytesStart::new("wsnt:TerminationTime")))?;
    writer.write_event(Event::Text(BytesText::new(termination_time)))?;
    writer.write_event(Event::End(BytesEnd::new("wsnt:TerminationTime")))?;
    writer.write_event(Event::End(BytesEnd::new("tev:Renew")))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::xml(e)))?;
    Envelope::new(RENEW_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Builds Unsubscribe.
pub fn unsubscribe_request(message_id: impl Into<String>) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tev:Unsubscribe");
    body.push_attribute(("xmlns:tev", EVENTS_NS));
    writer.write_event(Event::Empty(body))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::xml(e)))?;
    Envelope::new(UNSUBSCRIBE_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Parses CreatePullPointSubscriptionResponse.
pub fn parse_create_pull_point_response(
    xml: &str,
    limits: &ParserLimits,
    policy: &XAddrPolicy,
) -> Result<PullPointSubscription, OnvifServiceError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut ctx = ParseContext::new(limits, xml)?;
    let mut sub = PullPointSubscription {
        subscription_reference: String::new(),
        termination_time: None,
        current_time: None,
    };

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
                    "Address" => sub.subscription_reference = text.trim().to_string(),
                    "TerminationTime" => {
                        sub.termination_time = Some(text.trim().to_string());
                    }
                    "CurrentTime" => sub.current_time = Some(text.trim().to_string()),
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

    if sub.subscription_reference.is_empty() {
        return Err(OnvifServiceError::MissingField(
            "SubscriptionReference/Address".into(),
        ));
    }
    let url = url::Url::parse(&sub.subscription_reference)
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::invalid_xaddr(e)))?;
    policy.validate(&url).map_err(OnvifServiceError::Onvif)?;
    Ok(sub)
}

/// Parses PullMessagesResponse into notifications (bounded).
pub fn parse_pull_messages_response(
    xml: &str,
    limits: &ParserLimits,
    max_messages: usize,
) -> Result<Vec<OnvifNotification>, OnvifServiceError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut ctx = ParseContext::new(limits, xml)?;
    let mut messages = Vec::new();
    let mut current: Option<OnvifNotification> = None;
    let mut in_notification = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e.name());
                if name == "NotificationMessage" {
                    if messages.len() >= max_messages {
                        return Err(OnvifServiceError::Onvif(
                            cheetah_onvif_core::OnvifError::limit_exceeded(
                                "pull messages exceed max_messages",
                            ),
                        ));
                    }
                    in_notification = true;
                    current = Some(OnvifNotification {
                        topic: String::new(),
                        utc_time: None,
                        property_operation: None,
                        extension_xml: None,
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
                if name == "NotificationMessage" {
                    if let Some(msg) = current.take() {
                        messages.push(msg);
                    }
                    in_notification = false;
                } else if in_notification && let Some(ref mut msg) = current {
                    match name.as_str() {
                        "Topic" => msg.topic = text.trim().to_string(),
                        "UtcTime" | "UtcTimeAttr" => {
                            msg.utc_time = Some(text.trim().to_string());
                        }
                        "PropertyOperation" => {
                            msg.property_operation = Some(text.trim().to_string());
                        }
                        "Source" | "Key" | "Data"
                            if parent.as_deref() == Some("Message")
                                || parent.as_deref() == Some("tt:Message") =>
                        {
                            // Keep a short raw marker rather than unbounded XML.
                            let snippet = text.trim();
                            if !snippet.is_empty() {
                                let existing = msg.extension_xml.get_or_insert_with(String::new);
                                let sep = if existing.is_empty() { 0 } else { 1 };
                                let available = 512usize.saturating_sub(existing.len());
                                let max_take =
                                    available.saturating_sub(sep).min(128).min(snippet.len());
                                if max_take > 0 {
                                    if !existing.is_empty() {
                                        existing.push(';');
                                    }
                                    let mut take = max_take;
                                    while take > 0 && !snippet.is_char_boundary(take) {
                                        take -= 1;
                                    }
                                    existing.push_str(&snippet[..take]);
                                }
                            }
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

    Ok(messages)
}

const MAX_VENDOR_TOPIC_BYTES: usize = 256;

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    debug_assert!(needle.is_ascii());
    if needle.is_empty() {
        return true;
    }
    let n = needle.len();
    haystack.as_bytes().windows(n).any(|window| {
        window
            .iter()
            .zip(needle.as_bytes().iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// Maps a known ONVIF topic to a stable northbound event type, or vendor fallback.
///
/// Matching is case-insensitive and does not allocate a lowercase copy of the
/// whole topic. Unknown topics are truncated to [`MAX_VENDOR_TOPIC_BYTES`] to
/// prevent unbounded event type strings.
pub fn normalize_topic(topic: &str) -> String {
    if contains_ignore_ascii_case(topic, "cellmotion")
        || contains_ignore_ascii_case(topic, "motion")
    {
        "device.motion_detected".into()
    } else if contains_ignore_ascii_case(topic, "tns1:device/trigger/digitalinput")
        || contains_ignore_ascii_case(topic, "digitalinput")
    {
        "device.digital_input".into()
    } else if contains_ignore_ascii_case(topic, "globalscenechange")
        || contains_ignore_ascii_case(topic, "videoloss")
        || contains_ignore_ascii_case(topic, "videosource")
    {
        "device.video_loss".into()
    } else {
        let truncated: String = topic.chars().take(MAX_VENDOR_TOPIC_BYTES).collect();
        format!("vendor.onvif:{truncated}")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use cheetah_onvif_core::discovery::XAddrPolicy;

    #[test]
    fn create_pull_point_request_has_timeout() {
        let xml = create_pull_point_subscription_request("PT60S", "urn:uuid:1").unwrap();
        assert!(xml.contains("CreatePullPointSubscription"));
        assert!(xml.contains("PT60S"));
    }

    #[test]
    fn renew_request_declares_wsnt_namespace_and_termination_time() {
        let xml = renew_request("PT60S", "urn:uuid:1").unwrap();
        assert!(xml.contains("tev:Renew"));
        assert!(xml.contains("xmlns:wsnt=\"http://docs.oasis-open.org/wsn/b-2\""));
        assert!(xml.contains("<wsnt:TerminationTime>PT60S</wsnt:TerminationTime>"));
    }

    #[test]
    fn normalize_motion_topic() {
        assert_eq!(
            normalize_topic("tns1:RuleEngine/CellMotionDetector/Motion"),
            "device.motion_detected"
        );
        assert!(normalize_topic("tns1:Vendor/Custom").starts_with("vendor.onvif:"));
    }

    #[test]
    fn normalize_topic_is_case_insensitive() {
        assert_eq!(
            normalize_topic("TNS1:RuleEngine/CellMotionDetector/MOTION"),
            "device.motion_detected"
        );
        assert_eq!(
            normalize_topic("tns1:Device/Trigger/DigitalInput"),
            "device.digital_input"
        );
        assert_eq!(
            normalize_topic("tns1:VideoSource/GlobalSceneChange"),
            "device.video_loss"
        );
    }

    #[test]
    fn normalize_topic_truncates_long_unknown_topic() {
        let long = "x".repeat(1000);
        let normalized = normalize_topic(&long);
        assert!(normalized.starts_with("vendor.onvif:"));
        assert_eq!(
            normalized.len(),
            "vendor.onvif:".len() + MAX_VENDOR_TOPIC_BYTES
        );
    }

    #[test]
    fn parse_subscription_reference() {
        let xml = r#"
        <s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
          <s:Body>
            <tev:CreatePullPointSubscriptionResponse xmlns:tev="http://www.onvif.org/ver10/events/wsdl">
              <tev:SubscriptionReference>
                <a:Address xmlns:a="http://www.w3.org/2005/08/addressing">http://192.0.2.10/onvif/Events/PullPoint</a:Address>
              </tev:SubscriptionReference>
              <wsnt:CurrentTime xmlns:wsnt="http://docs.oasis-open.org/wsn/b-2">2020-01-01T00:00:00Z</wsnt:CurrentTime>
              <wsnt:TerminationTime xmlns:wsnt="http://docs.oasis-open.org/wsn/b-2">2020-01-01T00:01:00Z</wsnt:TerminationTime>
            </tev:CreatePullPointSubscriptionResponse>
          </s:Body>
        </s:Envelope>"#;
        let policy = XAddrPolicy::default().with_allow_private(true);
        let sub = parse_create_pull_point_response(xml, &ParserLimits::default(), &policy).unwrap();
        assert!(sub.subscription_reference.contains("192.0.2.10"));
    }

    #[test]
    fn parse_pull_messages_truncates_multibyte_source_on_char_boundary() {
        // 50 three-byte Chinese characters = 150 bytes; byte 128 is not a char
        // boundary, so the old byte-slice would panic. Char-boundary truncation
        // must keep a prefix that ends exactly on a character boundary.
        let source = "中".repeat(50);
        let xml = format!(
            r#"
        <s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
          <s:Body>
            <tev:PullMessagesResponse xmlns:tev="http://www.onvif.org/ver10/events/wsdl">
              <tev:NotificationMessage>
                <wsnt:Message xmlns:wsnt="http://docs.oasis-open.org/wsn/b-2">
                  <tt:Message xmlns:tt="http://www.onvif.org/ver10/schema">
                    <tt:Source>{source}</tt:Source>
                  </tt:Message>
                </wsnt:Message>
              </tev:NotificationMessage>
            </tev:PullMessagesResponse>
          </s:Body>
        </s:Envelope>"#
        );
        let messages = parse_pull_messages_response(&xml, &ParserLimits::default(), 10).unwrap();
        assert_eq!(messages.len(), 1);
        let ext = messages[0].extension_xml.as_deref().unwrap_or("");
        assert!(!ext.is_empty());
        // Truncated prefix must be valid UTF-8 and not exceed the 128-byte window.
        assert!(ext.len() <= 128);
        assert!(String::from_utf8(ext.as_bytes().to_vec()).is_ok());
    }
}

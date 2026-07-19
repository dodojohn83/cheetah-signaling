//! ONVIF Events PullPoint request builders and response parsers.

use crate::config::ParserLimits;
use crate::error::OnvifModuleError;
use crate::services::parse::{ParseContext, local_name};
use cheetah_onvif_core::discovery::XAddrPolicy;
use cheetah_onvif_core::soap::Envelope;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;

const EVENTS_NS: &str = "http://www.onvif.org/ver10/events/wsdl";
const CREATE_PULLPOINT_ACTION: &str =
    "http://www.onvif.org/ver10/events/wsdl/CreatePullPointSubscription";
const PULL_MESSAGES_ACTION: &str =
    "http://www.onvif.org/ver10/events/wsdl/PullPointSubscription/PullMessages";
const RENEW_ACTION: &str = "http://www.onvif.org/ver10/events/wsdl/PullPointSubscription/Renew";
const UNSUBSCRIBE_ACTION: &str =
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
) -> Result<String, OnvifModuleError> {
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
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(CREATE_PULLPOINT_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Builds PullMessages.
pub fn pull_messages_request(
    timeout: &str,
    message_limit: u32,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
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
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(PULL_MESSAGES_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Builds Renew with a termination time.
pub fn renew_request(
    termination_time: &str,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tev:Renew");
    body.push_attribute(("xmlns:tev", EVENTS_NS));
    writer.write_event(Event::Start(body))?;
    writer.write_event(Event::Start(BytesStart::new("wsnt:TerminationTime")))?;
    writer.write_event(Event::Text(BytesText::new(termination_time)))?;
    writer.write_event(Event::End(BytesEnd::new("wsnt:TerminationTime")))?;
    writer.write_event(Event::End(BytesEnd::new("tev:Renew")))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(RENEW_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Builds Unsubscribe.
pub fn unsubscribe_request(message_id: impl Into<String>) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut body = BytesStart::new("tev:Unsubscribe");
    body.push_attribute(("xmlns:tev", EVENTS_NS));
    writer.write_event(Event::Empty(body))?;
    let body = String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))?;
    Envelope::new(UNSUBSCRIBE_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Parses CreatePullPointSubscriptionResponse.
pub fn parse_create_pull_point_response(
    xml: &str,
    limits: &ParserLimits,
    policy: &XAddrPolicy,
) -> Result<PullPointSubscription, OnvifModuleError> {
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
                return Err(OnvifModuleError::Onvif(
                    cheetah_onvif_core::OnvifError::Xml(e.to_string()),
                ));
            }
            _ => {}
        }
    }

    if sub.subscription_reference.is_empty() {
        return Err(OnvifModuleError::MissingField(
            "SubscriptionReference/Address".into(),
        ));
    }
    let url = url::Url::parse(&sub.subscription_reference).map_err(|e| {
        OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::InvalidXAddr(e.to_string()))
    })?;
    policy.validate(&url).map_err(OnvifModuleError::Onvif)?;
    Ok(sub)
}

/// Parses PullMessagesResponse into notifications (bounded).
pub fn parse_pull_messages_response(
    xml: &str,
    limits: &ParserLimits,
    max_messages: usize,
) -> Result<Vec<OnvifNotification>, OnvifModuleError> {
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
                        return Err(OnvifModuleError::Onvif(
                            cheetah_onvif_core::OnvifError::LimitExceeded(
                                "pull messages exceed max_messages".into(),
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
                } else if in_notification {
                    if let Some(ref mut msg) = current {
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
                                    let existing =
                                        msg.extension_xml.get_or_insert_with(String::new);
                                    if existing.len() < 512 {
                                        if !existing.is_empty() {
                                            existing.push(';');
                                        }
                                        existing.push_str(&snippet[..snippet.len().min(128)]);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
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

    Ok(messages)
}

/// Maps a known ONVIF topic to a stable northbound event type, or vendor fallback.
pub fn normalize_topic(topic: &str) -> String {
    let lower = topic.to_ascii_lowercase();
    if lower.contains("motion") {
        "device.motion_detected".into()
    } else if lower.contains("cellmotion") {
        "device.motion_detected".into()
    } else if lower.contains("digitalinput") || lower.contains("tns1:device/trigger/digitalinput") {
        "device.digital_input".into()
    } else if lower.contains("globalscenechange")
        || lower.contains("videoloss")
        || lower.contains("videosource")
    {
        "device.video_loss".into()
    } else {
        format!("vendor.onvif:{topic}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheetah_onvif_core::discovery::XAddrPolicy;

    #[test]
    fn create_pull_point_request_has_timeout() {
        let xml = create_pull_point_subscription_request("PT60S", "urn:uuid:1").unwrap();
        assert!(xml.contains("CreatePullPointSubscription"));
        assert!(xml.contains("PT60S"));
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
}

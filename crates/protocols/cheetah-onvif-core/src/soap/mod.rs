//! SOAP 1.2 envelope helpers and fault parsing.

use crate::error::{OnvifError, OnvifResult};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::{Cursor, Write as _};

const SOAP_ENVELOPE: &str = "http://www.w3.org/2003/05/soap-envelope";
const WSA: &str = "http://www.w3.org/2005/08/addressing";

/// A lightweight SOAP 1.2 envelope builder.
#[derive(Clone, Debug)]
pub struct Envelope {
    action: String,
    body: String,
    message_id: Option<String>,
    security_header: Option<String>,
}

impl Envelope {
    /// Creates a new envelope for the given SOAP action and body XML fragment.
    pub fn new(action: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            body: body.into(),
            message_id: None,
            security_header: None,
        }
    }

    /// Sets the WS-Addressing MessageID.
    pub fn with_message_id(mut self, id: impl Into<String>) -> Self {
        self.message_id = Some(id.into());
        self
    }

    /// Sets a raw `wsse:Security` header fragment to include in the SOAP header.
    ///
    /// The supplied string must be a complete XML element (including its own
    /// namespace declaration); it is written verbatim into the header so that
    /// callers can inject `UsernameToken`s built by `cheetah-onvif-core`.
    pub fn with_security_header(mut self, xml: impl Into<String>) -> Self {
        self.security_header = Some(xml.into());
        self
    }

    /// Builds the envelope XML.
    pub fn build(&self) -> OnvifResult<String> {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = Writer::new(&mut cursor);

        writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

        let mut envelope = BytesStart::new("s:Envelope");
        envelope.push_attribute(("xmlns:s", SOAP_ENVELOPE));
        envelope.push_attribute(("xmlns:a", WSA));
        writer.write_event(Event::Start(envelope))?;

        writer.write_event(Event::Start(BytesStart::new("s:Header")))?;
        writer.write_event(Event::Start(BytesStart::new("a:Action")))?;
        writer.write_event(Event::Text(BytesText::new(&self.action)))?;
        writer.write_event(Event::End(BytesEnd::new("a:Action")))?;
        if let Some(ref id) = self.message_id {
            writer.write_event(Event::Start(BytesStart::new("a:MessageID")))?;
            writer.write_event(Event::Text(BytesText::new(id)))?;
            writer.write_event(Event::End(BytesEnd::new("a:MessageID")))?;
        }
        if let Some(ref security) = self.security_header {
            writer.get_mut().write_all(security.as_bytes())?;
        }
        writer.write_event(Event::End(BytesEnd::new("s:Header")))?;

        writer.write_event(Event::Start(BytesStart::new("s:Body")))?;
        // The body fragment is already valid XML produced by another typed
        // builder; write it directly so nested elements are preserved.
        writer.get_mut().write_all(self.body.as_bytes())?;
        writer.write_event(Event::End(BytesEnd::new("s:Body")))?;

        writer.write_event(Event::End(BytesEnd::new("s:Envelope")))?;

        String::from_utf8(cursor.into_inner()).map_err(|e| OnvifError::Xml(e.to_string()))
    }
}

/// A parsed SOAP fault.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Fault {
    /// Fault code QName.
    pub code: String,
    /// Human-readable fault reason.
    pub reason: String,
    /// Optional subcode.
    pub subcode: Option<String>,
    /// Optional detail string.
    pub detail: Option<String>,
}

/// Parses a SOAP `Fault` from an XML response.
pub fn parse_fault(xml: &str) -> OnvifResult<Fault> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut fault = Fault::default();
    let mut stack: Vec<String> = Vec::new();
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                stack.push(local_name(&e.name()));
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(&e.name());
                if let Some(parent) = stack.last() {
                    handle_end(parent, &name, "", &mut fault);
                }
            }
            Ok(Event::Text(e)) => {
                text.push_str(&e.xml10_content().unwrap_or_default());
            }
            Ok(Event::End(e)) => {
                let name = local_name(&e.name());
                stack.pop();
                if let Some(parent) = stack.last() {
                    handle_end(parent, &name, &text, &mut fault);
                }
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(OnvifError::Xml(e.to_string())),
            _ => {}
        }
    }

    Ok(fault)
}

fn handle_end(parent: &str, name: &str, text: &str, fault: &mut Fault) {
    let trimmed = text.trim();
    match (parent, name) {
        ("Code", "Value") if fault.code.is_empty() => {
            fault.code = trimmed.to_string();
        }
        ("Subcode", "Value") => {
            fault.subcode = Some(trimmed.to_string());
        }
        ("Reason", "Text") if fault.reason.is_empty() => {
            fault.reason = trimmed.to_string();
        }
        ("Fault", "Detail") => {
            fault.detail = Some(trimmed.to_string());
        }
        _ => {}
    }
}

fn local_name(name: &quick_xml::name::QName<'_>) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn envelope_includes_action_and_body() {
        let envelope = Envelope::new(
            "http://www.onvif.org/ver10/device/wsdl/GetDeviceInformation",
            "<tds:GetDeviceInformation/>",
        )
        .with_message_id("urn:uuid:msg-1");
        let xml = envelope.build().unwrap();
        assert!(xml.contains("<s:Envelope"));
        assert!(xml.contains(
            "<a:Action>http://www.onvif.org/ver10/device/wsdl/GetDeviceInformation</a:Action>"
        ));
        assert!(xml.contains("<a:MessageID>urn:uuid:msg-1</a:MessageID>"));
        assert!(xml.contains("<tds:GetDeviceInformation/>"));
    }

    #[test]
    fn parse_fault_extracts_code_and_reason() {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <s:Fault>
      <s:Code>
        <s:Value>s:Sender</s:Value>
        <s:Subcode>
          <s:Value>ter:InvalidArgVal</s:Value>
        </s:Subcode>
      </s:Code>
      <s:Reason>
        <s:Text xml:lang="en">Invalid argument</s:Text>
      </s:Reason>
      <s:Detail>device id missing</s:Detail>
    </s:Fault>
  </s:Body>
</s:Envelope>"#;
        let fault = parse_fault(xml).unwrap();
        assert_eq!(fault.code, "s:Sender");
        assert_eq!(fault.subcode, Some("ter:InvalidArgVal".to_string()));
        assert_eq!(fault.reason, "Invalid argument");
        assert_eq!(fault.detail, Some("device id missing".to_string()));
    }
}

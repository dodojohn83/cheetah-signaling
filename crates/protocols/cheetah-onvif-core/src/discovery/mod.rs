//! WS-Discovery message model and serializer/parser.
//!
//! All network side effects (multicast send/recv, interface selection) belong in
//! the driver crate. This module only builds and parses the SOAP-over-UDP
//! payloads. All timestamps are injected by the caller.

use crate::error::{OnvifError, OnvifResult};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use std::io::Cursor;

mod limits;
mod parser;
mod types;
mod xaddr;

pub use limits::{
    DiscoveryLimits, DiscoveryRateLimiter, LimitTracker, RateLimitConfig, check_datagram_size,
};
pub use types::{
    Bye, EndpointReference, Hello, Probe, ProbeMatch, ProbeMatches, Resolve, ResolveMatch,
    ResolveMatches, Scopes, XAddrs,
};
pub use xaddr::{XAddrPolicy, filter_xaddrs};

const SOAP_ENVELOPE: &str = "http://www.w3.org/2003/05/soap-envelope";
const WSA: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing";
const WSD: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery";

/// Application message ID for WS-Discovery messages, typically a UUID URN.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppId(pub String);

impl AppId {
    /// Creates an `AppId` from the supplied string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Builds a WS-Discovery `Probe` SOAP envelope.
pub fn build_probe(
    app_id: &AppId,
    types: &[String],
    scopes: Option<&Scopes>,
) -> OnvifResult<String> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);

    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    let mut envelope = BytesStart::new("s:Envelope");
    envelope.push_attribute(("xmlns:s", SOAP_ENVELOPE));
    envelope.push_attribute(("xmlns:a", WSA));
    envelope.push_attribute(("xmlns:d", WSD));
    writer.write_event(Event::Start(envelope))?;

    writer.write_event(Event::Start(BytesStart::new("s:Header")))?;
    write_header_element(&mut writer, "a:MessageID", &app_id.0)?;
    write_header_element(
        &mut writer,
        "a:To",
        "urn:docs-oasis-open:ws-dd:ns:discovery:2009:01",
    )?;
    write_header_element(
        &mut writer,
        "a:Action",
        "http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe",
    )?;
    writer.write_event(Event::End(BytesEnd::new("s:Header")))?;

    writer.write_event(Event::Start(BytesStart::new("s:Body")))?;
    let mut probe = BytesStart::new("d:Probe");
    probe.push_attribute(("xmlns:d", WSD));
    writer.write_event(Event::Start(probe))?;

    if !types.is_empty() {
        let mut types_elem = BytesStart::new("d:Types");
        types_elem.push_attribute(("xmlns:dp0", "http://www.onvif.org/ver10/network/wsdl"));
        writer.write_event(Event::Start(types_elem))?;
        writer.write_event(Event::Text(BytesText::new(&types.join(" "))))?;
        writer.write_event(Event::End(BytesEnd::new("d:Types")))?;
    }

    if let Some(scopes) = scopes {
        writer.write_event(Event::Start(BytesStart::new("d:Scopes")))?;
        writer.write_event(Event::Text(BytesText::new(&scopes.0.join(" "))))?;
        writer.write_event(Event::End(BytesEnd::new("d:Scopes")))?;
    }

    writer.write_event(Event::End(BytesEnd::new("d:Probe")))?;
    writer.write_event(Event::End(BytesEnd::new("s:Body")))?;
    writer.write_event(Event::End(BytesEnd::new("s:Envelope")))?;

    String::from_utf8(cursor.into_inner()).map_err(|e| OnvifError::Xml(e.to_string()))
}

/// Builds a WS-Discovery `Resolve` request for a known endpoint reference.
pub fn build_resolve(
    app_id: &AppId,
    endpoint_reference: &EndpointReference,
) -> OnvifResult<String> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);

    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    let mut envelope = BytesStart::new("s:Envelope");
    envelope.push_attribute(("xmlns:s", SOAP_ENVELOPE));
    envelope.push_attribute(("xmlns:a", WSA));
    envelope.push_attribute(("xmlns:d", WSD));
    writer.write_event(Event::Start(envelope))?;

    writer.write_event(Event::Start(BytesStart::new("s:Header")))?;
    write_header_element(&mut writer, "a:MessageID", &app_id.0)?;
    write_header_element(
        &mut writer,
        "a:To",
        "urn:docs-oasis-open:ws-dd:ns:discovery:2009:01",
    )?;
    write_header_element(
        &mut writer,
        "a:Action",
        "http://schemas.xmlsoap.org/ws/2005/04/discovery/Resolve",
    )?;
    writer.write_event(Event::End(BytesEnd::new("s:Header")))?;

    writer.write_event(Event::Start(BytesStart::new("s:Body")))?;
    let mut resolve = BytesStart::new("d:Resolve");
    resolve.push_attribute(("xmlns:d", WSD));
    writer.write_event(Event::Start(resolve))?;

    write_header_element(&mut writer, "a:EndpointReference", &endpoint_reference.0)?;

    writer.write_event(Event::End(BytesEnd::new("d:Resolve")))?;
    writer.write_event(Event::End(BytesEnd::new("s:Body")))?;
    writer.write_event(Event::End(BytesEnd::new("s:Envelope")))?;

    String::from_utf8(cursor.into_inner()).map_err(|e| OnvifError::Xml(e.to_string()))
}

fn write_header_element<W: std::io::Write>(
    writer: &mut Writer<W>,
    name: &str,
    value: &str,
) -> OnvifResult<()> {
    writer.write_event(Event::Start(BytesStart::new(name)))?;
    writer.write_event(Event::Text(BytesText::new(value)))?;
    writer.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

/// Parses a WS-Discovery `ProbeMatches` response.
///
/// `discovered_at` is a monotonic or wall-clock second timestamp supplied by
/// the driver. Uses default discovery limits.
pub fn parse_probe_matches(xml: &str, discovered_at: u64) -> OnvifResult<ProbeMatches> {
    parser::parse_probe_matches(xml, discovered_at, &DiscoveryLimits::default())
}

/// Parses a WS-Discovery `ProbeMatches` response with explicit limits.
pub fn parse_probe_matches_with_limits(
    xml: &str,
    discovered_at: u64,
    limits: &DiscoveryLimits,
) -> OnvifResult<ProbeMatches> {
    parser::parse_probe_matches(xml, discovered_at, limits)
}

/// Parses a WS-Discovery `Hello` or `Bye` announcement.
pub fn parse_hello_bye(xml: &str, discovered_at: u64) -> OnvifResult<EitherHelloBye> {
    parser::parse_hello_bye(xml, discovered_at, &DiscoveryLimits::default())
}

/// Parses a WS-Discovery `Hello` or `Bye` announcement with explicit limits.
pub fn parse_hello_bye_with_limits(
    xml: &str,
    discovered_at: u64,
    limits: &DiscoveryLimits,
) -> OnvifResult<EitherHelloBye> {
    parser::parse_hello_bye(xml, discovered_at, limits)
}

/// Parses a WS-Discovery `ResolveMatches` response.
pub fn parse_resolve_matches(xml: &str, discovered_at: u64) -> OnvifResult<ResolveMatches> {
    parser::parse_resolve_matches(xml, discovered_at, &DiscoveryLimits::default())
}

/// Parses a WS-Discovery `ResolveMatches` response with explicit limits.
pub fn parse_resolve_matches_with_limits(
    xml: &str,
    discovered_at: u64,
    limits: &DiscoveryLimits,
) -> OnvifResult<ResolveMatches> {
    parser::parse_resolve_matches(xml, discovered_at, limits)
}

/// Either a `Hello` or `Bye` announcement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EitherHelloBye {
    /// A presence announcement.
    Hello(Hello),
    /// A departure announcement.
    Bye(Bye),
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn probe_builds_valid_xml() {
        let app_id = AppId::new("urn:uuid:test");
        let xml = build_probe(
            &app_id,
            &["dp0:NetworkVideoTransmitter".to_string()],
            Some(&Scopes(vec![
                "onvif://www.onvif.org/type/video_encoder".to_string(),
            ])),
        )
        .unwrap();
        assert!(xml.contains("<s:Envelope"));
        assert!(xml.contains("<a:MessageID>urn:uuid:test</a:MessageID>"));
        assert!(xml.contains("<d:Types"));
        assert!(xml.contains("dp0:NetworkVideoTransmitter"));
        assert!(xml.contains("<d:Scopes"));
    }

    #[test]
    fn probe_matches_roundtrip() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:a="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery">
  <s:Header>
    <a:RelatesTo>urn:uuid:probe</a:RelatesTo>
  </s:Header>
  <s:Body>
    <d:ProbeMatches>
      <d:ProbeMatch>
        <a:EndpointReference>urn:uuid:device-1</a:EndpointReference>
        <d:Types>dp0:NetworkVideoTransmitter</d:Types>
        <d:XAddrs>http://192.168.1.10/onvif/device_service</d:XAddrs>
        <d:MetadataVersion>1</d:MetadataVersion>
      </d:ProbeMatch>
    </d:ProbeMatches>
  </s:Body>
</s:Envelope>"#;
        let matches = parse_probe_matches(xml, 42).unwrap();
        assert_eq!(matches.relates_to, "urn:uuid:probe");
        assert_eq!(matches.matches.len(), 1);
        assert_eq!(matches.matches[0].endpoint_reference.0, "urn:uuid:device-1");
        assert_eq!(matches.matches[0].discovered_at, 42);
        assert_eq!(matches.matches[0].metadata_version, 1);
    }

    #[test]
    fn probe_matches_parses_nested_endpoint_reference() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:a="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery">
  <s:Header>
    <a:RelatesTo>urn:uuid:probe</a:RelatesTo>
  </s:Header>
  <s:Body>
    <d:ProbeMatches>
      <d:ProbeMatch>
        <a:EndpointReference>
          <a:Address>urn:uuid:nested-device</a:Address>
        </a:EndpointReference>
        <d:Types>dp0:NetworkVideoTransmitter</d:Types>
        <d:XAddrs>http://192.168.1.10/onvif/device_service</d:XAddrs>
        <d:MetadataVersion>1</d:MetadataVersion>
      </d:ProbeMatch>
    </d:ProbeMatches>
  </s:Body>
</s:Envelope>"#;
        let matches = parse_probe_matches(xml, 42).unwrap();
        assert_eq!(
            matches.matches[0].endpoint_reference.0,
            "urn:uuid:nested-device"
        );
    }

    #[test]
    fn hello_bye_parsing() {
        let hello = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:a="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery">
  <s:Body>
    <d:Hello>
      <a:EndpointReference>
        <a:Address>urn:uuid:hello-device</a:Address>
      </a:EndpointReference>
      <d:XAddrs>http://10.0.0.1/onvif</d:XAddrs>
      <d:MetadataVersion>5</d:MetadataVersion>
    </d:Hello>
  </s:Body>
</s:Envelope>"#;
        match parse_hello_bye(hello, 100).unwrap() {
            EitherHelloBye::Hello(h) => {
                assert_eq!(h.endpoint_reference.0, "urn:uuid:hello-device");
                assert_eq!(h.metadata_version, 5);
                assert_eq!(h.discovered_at, 100);
            }
            _ => panic!("expected Hello"),
        }
    }

    #[test]
    fn filter_xaddrs_rejects_loopback() {
        let addrs = vec!["http://127.0.0.1/onvif".to_string()];
        assert!(filter_xaddrs(&addrs, false).is_empty());
        assert_eq!(filter_xaddrs(&addrs, true).len(), 1);
    }

    #[test]
    fn filter_xaddrs_rejects_non_http_scheme() {
        let addrs = vec!["ftp://192.168.1.1/onvif".to_string()];
        assert!(filter_xaddrs(&addrs, true).is_empty());
    }

    #[test]
    fn filter_xaddrs_keeps_usable_addresses() {
        let addrs = vec![
            "http://192.0.2.1/onvif".to_string(),
            "http://127.0.0.1/onvif".to_string(),
            "http://192.168.1.1/onvif".to_string(),
        ];
        let filtered = filter_xaddrs(&addrs, false);
        assert_eq!(filtered, vec!["http://192.0.2.1/onvif".to_string()]);
    }
}

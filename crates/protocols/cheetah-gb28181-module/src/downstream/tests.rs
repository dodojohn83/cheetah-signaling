#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
};
use secrecy::SecretString;

const REALM: &str = "example.com";
const PLATFORM_ID: &str = "34020000002000000001";
const SERVER_SECRET: &[u8] = b"server-secret-must-be-32-bytes-long";

fn make_config() -> DownstreamConfig {
    DownstreamConfig::new(
        REALM,
        "sip:34020000002000000001@example.com",
        REALM,
        SERVER_SECRET.to_vec(),
    )
    .unwrap()
    .with_auth_policy(crate::config::AuthPolicy::ChallengeOptional)
}

fn make_register(expires: u32) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKabc"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{PLATFORM_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{PLATFORM_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 REGISTER".to_string()));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!(
            "<sip:{PLATFORM_ID}@192.168.1.100:5060>;expires={expires}"
        )),
    );
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    SipMessage::Request {
        line: RequestLine::new(
            Method::Register,
            SipUri::parse(format!("sip:{PLATFORM_ID}@{REALM}")).unwrap(),
        ),
        headers,
        body: Vec::new(),
    }
}

fn make_message(body: &[u8]) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKdef"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{PLATFORM_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{PLATFORM_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 MESSAGE".to_string()));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml".to_string()),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    SipMessage::Request {
        line: RequestLine::new(
            Method::Message,
            SipUri::parse(format!("sip:{PLATFORM_ID}@{REALM}")).unwrap(),
        ),
        headers,
        body: body.to_vec(),
    }
}

#[allow(clippy::unnecessary_wraps)]
fn credential_provider(_id: &DeviceId) -> Option<SecretString> {
    Some(SecretString::from("secret"))
}

#[test]
fn register_challenge_optional_returns_200_and_emits_event() {
    let mut downstream = Gb28181Downstream::new(make_config(), credential_provider).unwrap();
    let source = "192.168.1.100:5060".parse().unwrap();
    let outputs = downstream
        .process(DownstreamInput::Sip {
            source,
            now: 0,
            message: make_register(3600),
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    let response = match &outputs[0] {
        DownstreamOutput::SendResponse(r) => r,
        _ => panic!("expected SendResponse"),
    };
    assert!(matches!(response, SipMessage::Response { line, .. } if line.code == 200));
    assert!(matches!(
        outputs[1],
        DownstreamOutput::EmitEvent(Gb28181Event::DeviceRegistered { .. })
    ));
}

#[test]
fn keepalive_from_registered_platform_emits_event_and_200() {
    let mut downstream = Gb28181Downstream::new(make_config(), credential_provider).unwrap();
    let source = "192.168.1.100:5060".parse().unwrap();
    downstream
        .process(DownstreamInput::Sip {
            source,
            now: 0,
            message: make_register(3600),
        })
        .unwrap();

    let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000002000000001</DeviceID>
    <Status>OK</Status>
</Notify>"#;
    let outputs = downstream
        .process(DownstreamInput::Sip {
            source,
            now: 1,
            message: make_message(body),
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    assert!(matches!(
        outputs[0],
        DownstreamOutput::EmitEvent(Gb28181Event::Keepalive { .. })
    ));
    assert!(matches!(outputs[1], DownstreamOutput::SendResponse(_)));
}

#[test]
fn query_catalog_command_builds_outbound_message() {
    let mut downstream = Gb28181Downstream::new(make_config(), credential_provider).unwrap();
    let source = "192.168.1.100:5060".parse().unwrap();
    downstream
        .process(DownstreamInput::Sip {
            source,
            now: 0,
            message: make_register(3600),
        })
        .unwrap();

    let platform_id = DeviceId::new(PLATFORM_ID).unwrap();
    let outputs = downstream
        .process(DownstreamInput::Command {
            now: 1,
            command: DownstreamCommand::QueryCatalog {
                platform_id,
                sn: "7".to_string(),
            },
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let (request, destination) = match &outputs[0] {
        DownstreamOutput::SendRequest(r, d) => (r, d),
        _ => panic!("expected SendRequest"),
    };
    assert_eq!(destination, &source);
    assert!(matches!(request, SipMessage::Request { line, .. } if line.method == Method::Message));
}

#[test]
fn unregistered_message_returns_403() {
    let mut downstream = Gb28181Downstream::new(make_config(), credential_provider).unwrap();
    let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000002000000001</DeviceID>
    <Status>OK</Status>
</Notify>"#;
    let outputs = downstream
        .process(DownstreamInput::Sip {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 0,
            message: make_message(body),
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    match &outputs[0] {
        DownstreamOutput::SendResponse(SipMessage::Response { line, .. }) => {
            assert_eq!(line.code, 403);
        }
        _ => panic!("expected 403 response"),
    };
}

//! Legal and illegal state-transition table tests for the GB28181 access
//! state machine (`GB4-TST-002`).
//!
//! The [`GbAccessMachine`] contract has no single explicit state enum; its
//! observable state is the per-device registration presence. These rows form an
//! explicit transition table over that presence: unauthenticated / authenticated
//! REGISTER, keepalive before and after registration, duplicate REGISTER,
//! expiry timeout, business messages from unregistered devices, and malformed
//! messages. Each row applies a single input to a freshly-prepared machine and
//! asserts the resulting SIP response code and emitted domain event. All timing
//! is driven by an explicit monotonic `now`; no real clock is used.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_gb28181_core::{
    GbAccessMachine, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
};
use cheetah_gb28181_module::{
    AccessInput, AccessOutput, AuthPolicy, CredentialError, CredentialProvider, DeviceId,
    Gb28181Access, Gb28181DomainConfig, Gb28181Event,
};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

const REALM: &str = "example.com";
const DEVICE_ID: &str = "34020000001320000001";
const PASSWORD: &str = "secret";
const SERVER_SECRET: &[u8] = b"server-secret-must-be-32-bytes-long";

fn source() -> std::net::SocketAddr {
    "192.168.1.100:5060".parse().unwrap()
}

fn required_config() -> Gb28181DomainConfig {
    Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec()).unwrap()
}

fn optional_config() -> Gb28181DomainConfig {
    Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional)
}

fn password_provider() -> impl CredentialProvider {
    |device: &DeviceId| -> Result<Option<SecretString>, CredentialError> {
        if device.as_ref() == DEVICE_ID {
            Ok(Some(SecretString::from(PASSWORD)))
        } else {
            Ok(None)
        }
    }
}

fn no_password_provider() -> impl CredentialProvider {
    |_: &DeviceId| -> Result<Option<SecretString>, CredentialError> { Ok(None) }
}

fn make_register(cseq: u32, expires: u32) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKabc"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-1"));
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{cseq} REGISTER")),
    );
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!(
            "<sip:{DEVICE_ID}@192.168.1.100:5060>;expires={expires}"
        )),
    );
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Request {
        line: RequestLine::new(
            Method::Register,
            SipUri::parse(format!("sip:{DEVICE_ID}@{REALM}")).unwrap(),
        ),
        headers,
        body: Vec::new(),
    }
}

fn make_message(device_id: &str, body: &[u8]) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKdef"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{device_id}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{device_id}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-2"));
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
            SipUri::parse(format!("sip:{device_id}@{REALM}")).unwrap(),
        ),
        headers,
        body: body.to_vec(),
    }
}

fn keepalive_body(device_id: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\"?>\n<Notify>\n<CmdType>Keepalive</CmdType>\n<SN>1</SN>\n<DeviceID>{device_id}</DeviceID>\n<Status>OK</Status>\n</Notify>"
    )
    .into_bytes()
}

fn malformed_body() -> Vec<u8> {
    b"<Notify><CmdType>Keepalive</CmdType>".to_vec()
}

fn add_authorization(request: &mut SipMessage, nonce: &str) {
    let a1 = format!("{DEVICE_ID}:{REALM}:{PASSWORD}");
    let ha1 = hex::encode(Sha256::digest(a1.as_bytes()));
    let a2 = format!("REGISTER:sip:{DEVICE_ID}@{REALM}");
    let ha2 = hex::encode(Sha256::digest(a2.as_bytes()));
    let a3 = format!("{ha1}:{nonce}:00000001:clientnonce:auth:{ha2}");
    let response = hex::encode(Sha256::digest(a3.as_bytes()));
    let value = format!(
        r##"Digest username="{DEVICE_ID}", realm="{REALM}", nonce="{nonce}", uri="sip:{DEVICE_ID}@{REALM}", response="{response}", cnonce="clientnonce", nc="00000001", qop="auth", algorithm="SHA-256""##
    );
    request
        .headers_mut()
        .append(HeaderName::Authorization, HeaderValue::new(value));
}

fn extract_nonce(header: &str) -> String {
    header
        .split(',')
        .find_map(|part| {
            part.trim()
                .strip_prefix("nonce=\"")
                .and_then(|v| v.split('\"').next())
                .map(String::from)
        })
        .expect("nonce in challenge")
}

fn process(
    access: &mut Gb28181Access<impl CredentialProvider>,
    now: u64,
    message: SipMessage,
) -> Vec<AccessOutput<Gb28181Event>> {
    access
        .process(AccessInput {
            source: source(),
            now,
            message,
        })
        .unwrap()
}

fn response_code(outputs: &[AccessOutput<Gb28181Event>]) -> Option<u16> {
    outputs.iter().find_map(|o| match o {
        AccessOutput::SendResponse(SipMessage::Response { line, .. }) => Some(line.code),
        _ => None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvKind {
    Registered,
    Unregistered,
    Keepalive,
}

fn has_event(outputs: &[AccessOutput<Gb28181Event>], kind: EvKind) -> bool {
    outputs.iter().any(|o| match o {
        AccessOutput::EmitEvent(Gb28181Event::DeviceRegistered { .. }) => {
            kind == EvKind::Registered
        }
        AccessOutput::EmitEvent(Gb28181Event::DeviceUnregistered { .. }) => {
            kind == EvKind::Unregistered
        }
        AccessOutput::EmitEvent(Gb28181Event::Keepalive { .. }) => kind == EvKind::Keepalive,
        _ => false,
    })
}

/// Registers `DEVICE_ID` on a fresh `ChallengeOptional` machine so a follow-up
/// transition observes a device that is already online.
fn registered_optional(expires: u32) -> Gb28181Access<impl CredentialProvider> {
    let mut access = Gb28181Access::new(optional_config(), no_password_provider()).unwrap();
    let outputs = process(&mut access, 1000, make_register(1, expires));
    assert_eq!(response_code(&outputs), Some(200));
    access
}

/// Authenticates `DEVICE_ID` through the digest challenge/response handshake and
/// returns the final REGISTER outputs.
fn authenticated_register() -> Vec<AccessOutput<Gb28181Event>> {
    let mut access = Gb28181Access::new(required_config(), password_provider()).unwrap();
    let mut request = make_register(1, 3600);
    let challenge_outputs = process(&mut access, 1000, request.clone());
    let AccessOutput::SendResponse(challenge) = &challenge_outputs[0] else {
        panic!("expected challenge response");
    };
    let www_auth = challenge
        .headers()
        .get(&HeaderName::WwwAuthenticate)
        .expect("WWW-Authenticate")
        .as_str();
    let nonce = extract_nonce(www_auth);
    add_authorization(&mut request, &nonce);
    process(&mut access, 1001, request)
}

type Row = (
    &'static str,
    Box<dyn Fn() -> Vec<AccessOutput<Gb28181Event>>>,
    Option<u16>,
    Option<EvKind>,
);

fn rows() -> Vec<Row> {
    vec![
        (
            "unauthenticated REGISTER (Required) -> 401",
            Box::new(|| {
                let mut access =
                    Gb28181Access::new(required_config(), password_provider()).unwrap();
                process(&mut access, 1000, make_register(1, 3600))
            }),
            Some(401),
            None,
        ),
        (
            "authenticated REGISTER -> 200 + DeviceRegistered",
            Box::new(authenticated_register),
            Some(200),
            Some(EvKind::Registered),
        ),
        (
            "keepalive before registration -> 403",
            Box::new(|| {
                let mut access =
                    Gb28181Access::new(optional_config(), no_password_provider()).unwrap();
                process(
                    &mut access,
                    1000,
                    make_message(DEVICE_ID, &keepalive_body(DEVICE_ID)),
                )
            }),
            Some(403),
            None,
        ),
        (
            "keepalive after registration -> 200 + Keepalive",
            Box::new(|| {
                let mut access = registered_optional(3600);
                process(
                    &mut access,
                    1001,
                    make_message(DEVICE_ID, &keepalive_body(DEVICE_ID)),
                )
            }),
            Some(200),
            Some(EvKind::Keepalive),
        ),
        (
            "duplicate REGISTER -> 200 + DeviceRegistered",
            Box::new(|| {
                let mut access = registered_optional(3600);
                process(&mut access, 1001, make_register(2, 3600))
            }),
            Some(200),
            Some(EvKind::Registered),
        ),
        (
            "expiry timeout tick -> DeviceUnregistered",
            Box::new(|| {
                let mut access = registered_optional(30);
                access.tick(1000 + 31).unwrap()
            }),
            None,
            Some(EvKind::Unregistered),
        ),
        (
            "business message from unregistered device -> 403",
            Box::new(|| {
                let mut access =
                    Gb28181Access::new(optional_config(), no_password_provider()).unwrap();
                // A different device that never registered.
                let other = "34020000001320000009";
                process(
                    &mut access,
                    1000,
                    make_message(other, &keepalive_body(other)),
                )
            }),
            Some(403),
            None,
        ),
        (
            "malformed message from registered device -> 400",
            Box::new(|| {
                let mut access = registered_optional(3600);
                process(
                    &mut access,
                    1001,
                    make_message(DEVICE_ID, &malformed_body()),
                )
            }),
            Some(400),
            None,
        ),
    ]
}

#[test]
fn access_transition_table() {
    for (name, run, expected_code, expected_event) in rows() {
        let outputs = run();
        assert_eq!(
            response_code(&outputs),
            expected_code,
            "[{name}] unexpected response code; outputs={outputs:?}"
        );
        if let Some(kind) = expected_event {
            assert!(
                has_event(&outputs, kind),
                "[{name}] expected event {kind:?}; outputs={outputs:?}"
            );
        }
    }
}

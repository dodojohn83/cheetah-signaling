//! Driver-level integration tests for the GB28181 endpoint route model,
//! NAT/`rport` policy and source-hijack regression (GB4-SIP-006).
//!
//! These drive the driver's transaction-user (the `Gb28181Access` machine) with
//! transport-shaped inputs and assert the resolved send target, plus one
//! real-UDP-socket test for symmetric response routing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use cheetah_gb28181_core::{
    AccessInput, GbAccessMachine, HeaderName, HeaderValue, Method, RequestLine, SipHeaders,
    SipMessage, SipUri,
};
use cheetah_gb28181_driver_tokio::Gb28181UdpDriver;
use cheetah_gb28181_driver_tokio::config::DriverConfig;
use cheetah_gb28181_driver_tokio::sink::{EventSink, NoOpEventSink};
use cheetah_gb28181_module::{
    AuthPolicy, CredentialError, CredentialProvider, DeviceId, Gb28181Access, Gb28181DomainConfig,
    Gb28181Event,
};
use common::{challenging_access, encode_register, parse_datagram};
use secrecy::SecretString;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

const REALM: &str = "example.com";
const DEVICE_ID: &str = "34020000001320000001";
const SERVER_SECRET: &[u8] = b"server-secret-must-be-32-bytes-long";

fn sock(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn device() -> DeviceId {
    DeviceId::new(DEVICE_ID).unwrap()
}

/// Challenge-optional access with no configured password: an accepted REGISTER
/// is the sanctioned (authenticated) route-update context.
fn optional_auth_access() -> Gb28181Access<impl CredentialProvider> {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider =
        |_device: &DeviceId| -> Result<Option<SecretString>, CredentialError> { Ok(None) };
    Gb28181Access::new(config, provider).unwrap()
}

fn register_message(via: &str, contact: &str, expires: u32) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(HeaderName::Via, HeaderValue::new(via));
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-reg"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 REGISTER"));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{contact}>;expires={expires}")),
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

fn keepalive_message(via: &str) -> SipMessage {
    let body = format!(
        "<?xml version=\"1.0\"?>\n<Notify>\n<CmdType>Keepalive</CmdType>\n<SN>1</SN>\n<DeviceID>{DEVICE_ID}</DeviceID>\n<Status>OK</Status>\n</Notify>"
    )
    .into_bytes();
    let mut headers = SipHeaders::new();
    headers.append(HeaderName::Via, HeaderValue::new(via));
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-msg"));
    headers.append(HeaderName::CSeq, HeaderValue::new("2 MESSAGE"));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );
    SipMessage::Request {
        line: RequestLine::new(
            Method::Message,
            SipUri::parse(format!("sip:{DEVICE_ID}@{REALM}")).unwrap(),
        ),
        headers,
        body,
    }
}

fn register(
    access: &mut Gb28181Access<impl CredentialProvider>,
    source: SocketAddr,
    via: &str,
    contact: &str,
) {
    access
        .process(AccessInput {
            source,
            now: 1000,
            message: register_message(via, contact, 3600),
        })
        .expect("register accepted");
}

#[test]
fn rport_policy_prefers_observed_source_over_private_contact() {
    // Device behind NAT: Via sent-by and Contact carry a private address, but
    // the request includes `;rport`, so the send target must be the public
    // observed source, not the unreachable private endpoint.
    let mut access = optional_auth_access();
    let observed = sock("203.0.113.9:41234");
    register(
        &mut access,
        observed,
        "SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bKnat;rport",
        "sip:34020000001320000001@10.0.0.5:5060",
    );
    assert_eq!(access.device_send_target(&device()), Some(observed));
}

#[test]
fn nat_rewrite_uses_public_contact_when_no_rport() {
    // No `rport`: the advertised public Contact endpoint wins over the observed
    // source.
    let mut access = optional_auth_access();
    register(
        &mut access,
        sock("203.0.113.9:41234"),
        "SIP/2.0/UDP 198.51.100.20:5062;branch=z9hG4bKpub",
        "sip:34020000001320000001@198.51.100.20:5062",
    );
    assert_eq!(
        access.device_send_target(&device()),
        Some(sock("198.51.100.20:5062"))
    );
}

#[test]
fn no_rport_and_unresolvable_contact_falls_back_to_observed_source() {
    // Domain-name Contact cannot be resolved in the Sans-I/O core, and no
    // `rport` was requested: fall back to the observed source.
    let mut access = optional_auth_access();
    let observed = sock("203.0.113.9:41234");
    register(
        &mut access,
        observed,
        "SIP/2.0/UDP device.example.com:5060;branch=z9hG4bKdns",
        "sip:34020000001320000001@device.example.com",
    );
    assert_eq!(access.device_send_target(&device()), Some(observed));
}

#[test]
fn authenticated_reregister_moves_endpoint() {
    // Endpoint drift across REGISTERs is allowed: an authenticated re-REGISTER
    // from a new source updates the stored send route.
    let mut access = optional_auth_access();
    let first = sock("203.0.113.9:41234");
    register(
        &mut access,
        first,
        "SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bK1;rport",
        "sip:34020000001320000001@10.0.0.5:5060",
    );
    assert_eq!(access.device_send_target(&device()), Some(first));

    let second = sock("198.51.100.77:52000");
    register(
        &mut access,
        second,
        "SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bK2;rport",
        "sip:34020000001320000001@10.0.0.5:5060",
    );
    assert_eq!(access.device_send_target(&device()), Some(second));
}

#[test]
fn keepalive_from_hijack_source_does_not_move_endpoint() {
    // Source-hijack regression: a keepalive/MESSAGE spoofed from a different
    // address must not move the stored send route.
    let mut access = optional_auth_access();
    let registered = sock("203.0.113.9:41234");
    register(
        &mut access,
        registered,
        "SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bK1;rport",
        "sip:34020000001320000001@10.0.0.5:5060",
    );
    assert_eq!(access.device_send_target(&device()), Some(registered));

    // Attacker keepalive from an unrelated source.
    access
        .process(AccessInput {
            source: sock("198.51.100.66:9999"),
            now: 1005,
            message: keepalive_message("SIP/2.0/UDP 198.51.100.66:9999;branch=z9hG4bKevil"),
        })
        .expect("keepalive processed");

    // Route unchanged: still points at the registered endpoint.
    assert_eq!(access.device_send_target(&device()), Some(registered));

    // A legitimate keepalive from the registered source also leaves it intact.
    access
        .process(AccessInput {
            source: registered,
            now: 1006,
            message: keepalive_message("SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bKok;rport"),
        })
        .expect("keepalive processed");
    assert_eq!(access.device_send_target(&device()), Some(registered));
}

#[tokio::test]
async fn register_response_is_routed_to_observed_source() {
    // End-to-end symmetric routing over a real UDP socket: the 401 response
    // must come back to the client's actual source regardless of the Via
    // sent-by carried in the request.
    let config = DriverConfig::new("127.0.0.1:0".parse().unwrap());
    let sink: Arc<dyn EventSink<Gb28181Event>> = Arc::new(NoOpEventSink);
    let (driver, local_addr) = Gb28181UdpDriver::bind(config, challenging_access(), sink)
        .await
        .expect("bind");
    let handle = tokio::spawn(driver.run());

    let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
    let client_addr = client.local_addr().unwrap();
    client
        .send_to(&encode_register(DEVICE_ID, 5060), local_addr)
        .await
        .expect("send");

    let mut buf = vec![0u8; 65535];
    let (len, source) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
        .await
        .expect("receive within timeout")
        .expect("recv_from");
    assert_eq!(source, local_addr);
    match parse_datagram(&buf[..len]) {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 401),
        _ => panic!("expected response"),
    }
    // The client received the response at its own bound address (symmetric).
    assert_eq!(client.local_addr().unwrap(), client_addr);

    handle.abort();
}

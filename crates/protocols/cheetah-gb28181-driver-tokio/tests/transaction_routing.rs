//! Transaction-layer and method-routing contract tests (GB4-SIP-002/003).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri, encode_message,
};
use cheetah_gb28181_driver_tokio::Gb28181UdpDriver;
use cheetah_gb28181_driver_tokio::config::DriverConfig;
use cheetah_gb28181_driver_tokio::sink::EventSink;
use cheetah_gb28181_module::{AuthPolicy, Gb28181Access, Gb28181Event};
use common::{parse_datagram, test_credential_provider, test_domain_config};
use secrecy::SecretString;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

/// Event sink that records every emitted event for assertions.
#[derive(Clone, Default)]
struct CountingSink {
    events: Arc<Mutex<Vec<Gb28181Event>>>,
}

impl EventSink<Gb28181Event> for CountingSink {
    fn emit(&self, event: Gb28181Event) {
        self.events.lock().unwrap().push(event);
    }
}

fn optional_auth_access(
    device_id: &str,
) -> Gb28181Access<impl cheetah_gb28181_module::CredentialProvider + 'static> {
    let domain = test_domain_config(AuthPolicy::ChallengeOptional);
    let mut passwords = HashMap::new();
    passwords.insert(device_id.to_string(), SecretString::new("ignored".into()));
    Gb28181Access::new(domain, test_credential_provider(passwords)).expect("access")
}

fn options_request(device_id: &str, port: u16) -> Vec<u8> {
    let uri = SipUri::parse(format!("sip:{device_id}@127.0.0.1")).expect("uri");
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "127.0.0.1", port, &format!("z9hG4bKopt{device_id}")).expect("via"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&uri, &format!("tag-{device_id}")).expect("from"),
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&uri));
    headers.append(
        HeaderName::CallId,
        HeaderValue::new(format!("call-opt-{device_id}")),
    );
    headers.append(HeaderName::CSeq, HeaderValue::cseq(1, Method::Options));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    encode_message(&SipMessage::Request {
        line: RequestLine::new(Method::Options, uri),
        headers,
        body: Vec::new(),
    })
}

/// A retransmitted REGISTER (same transaction key) must be answered from the
/// cached response without re-running the registrar, so exactly one
/// registration event is emitted while both requests are answered.
#[tokio::test]
async fn duplicate_register_is_absorbed_and_emits_single_event() {
    let device = "34020000001320000001";
    let config = DriverConfig::new("127.0.0.1:0".parse().unwrap());
    let sink = CountingSink::default();
    let (driver, local_addr) =
        Gb28181UdpDriver::bind(config, optional_auth_access(device), Arc::new(sink.clone()))
            .await
            .expect("bind");
    let handle = tokio::spawn(driver.run());

    let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
    let datagram = common::encode_register(device, 5060);

    let mut buf = vec![0u8; 65535];
    let mut codes = Vec::new();
    for _ in 0..2 {
        client.send_to(&datagram, local_addr).await.expect("send");
        let (len, _) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
            .await
            .expect("receive within timeout")
            .expect("recv_from");
        match parse_datagram(&buf[..len]) {
            SipMessage::Response { line, .. } => codes.push(line.code),
            _ => panic!("expected response"),
        }
    }

    assert_eq!(
        codes,
        vec![200, 200],
        "both requests answered with cached 200"
    );

    // Give the driver a moment to have emitted events.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let registered = sink
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, Gb28181Event::DeviceRegistered { .. }))
        .count();
    assert_eq!(
        registered, 1,
        "duplicate request must not re-run registration business logic"
    );

    handle.abort();
}

/// An OPTIONS request is routed to the transaction user and receives a SIP
/// response rather than being silently dropped, proving non-REGISTER methods
/// are dispatched through the driver.
#[tokio::test]
async fn options_is_routed_and_answered() {
    let device = "34020000001320000002";
    let config = DriverConfig::new("127.0.0.1:0".parse().unwrap());
    let (driver, local_addr) = Gb28181UdpDriver::bind(
        config,
        optional_auth_access(device),
        Arc::new(CountingSink::default()),
    )
    .await
    .expect("bind");
    let handle = tokio::spawn(driver.run());

    let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
    client
        .send_to(&options_request(device, 5060), local_addr)
        .await
        .expect("send");

    let mut buf = vec![0u8; 65535];
    let (len, _) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
        .await
        .expect("receive within timeout")
        .expect("recv_from");
    match parse_datagram(&buf[..len]) {
        SipMessage::Response { line, .. } => {
            // The registrar has no OPTIONS handler yet, so it answers 501; the
            // point of the test is that the method was routed and answered.
            assert!(line.code >= 200, "OPTIONS must receive a final response");
        }
        _ => panic!("expected response"),
    }

    handle.abort();
}

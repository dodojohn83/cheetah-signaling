//! UDP transport contract tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use cheetah_gb28181_core::{HeaderName, SipMessage};
use cheetah_gb28181_driver_tokio::config::DriverConfig;
use cheetah_gb28181_driver_tokio::sink::{EventSink, NoOpEventSink};
use cheetah_gb28181_driver_tokio::{DriverError, Gb28181UdpDriver};
use cheetah_gb28181_module::{AuthPolicy, Gb28181Access, Gb28181Event};
use common::{
    build_register_request, challenging_access, encode_register, parse_datagram,
    test_credential_provider, test_domain_config,
};
use secrecy::SecretString;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

#[tokio::test]
async fn register_without_credentials_gets_401_challenge() {
    let config = DriverConfig::new("127.0.0.1:0".parse().unwrap());
    let sink: Arc<dyn EventSink<Gb28181Event>> = Arc::new(NoOpEventSink);
    let (driver, local_addr) = Gb28181UdpDriver::bind(config, challenging_access(), sink)
        .await
        .expect("bind");

    let handle = tokio::spawn(driver.run());

    let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
    client
        .send_to(&encode_register("34020000001320000001", 5060), local_addr)
        .await
        .expect("send");

    let mut buf = vec![0u8; 65535];
    let (len, _source) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
        .await
        .expect("receive within timeout")
        .expect("recv_from");

    match parse_datagram(&buf[..len]) {
        SipMessage::Response { line, headers, .. } => {
            assert_eq!(line.code, 401);
            assert!(headers.get(&HeaderName::WwwAuthenticate).is_some());
        }
        _ => panic!("expected response"),
    }

    handle.abort();
}

#[tokio::test]
async fn register_with_optional_auth_gets_200_ok() {
    let config = DriverConfig::new("127.0.0.1:0".parse().unwrap());
    let domain = test_domain_config(AuthPolicy::ChallengeOptional);
    let mut passwords = HashMap::new();
    passwords.insert(
        "34020000001320000001".to_string(),
        SecretString::new("ignored".into()),
    );
    let access = Gb28181Access::new(domain, test_credential_provider(passwords)).expect("access");
    let sink: Arc<dyn EventSink<Gb28181Event>> = Arc::new(NoOpEventSink);
    let (driver, local_addr) = Gb28181UdpDriver::bind(config, access, sink)
        .await
        .expect("bind");

    let handle = tokio::spawn(driver.run());

    let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
    client
        .send_to(&encode_register("34020000001320000001", 5060), local_addr)
        .await
        .expect("send");

    let mut buf = vec![0u8; 65535];
    let (len, _source) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
        .await
        .expect("receive within timeout")
        .expect("recv_from");

    match parse_datagram(&buf[..len]) {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 200),
        _ => panic!("expected response"),
    }

    handle.abort();
}

#[tokio::test]
async fn oversized_datagram_is_rejected_without_crash() {
    let config = DriverConfig::new("127.0.0.1:0".parse().unwrap()).with_max_datagram_size(1500);
    let sink: Arc<dyn EventSink<Gb28181Event>> = Arc::new(NoOpEventSink);
    let (driver, local_addr) = Gb28181UdpDriver::bind(config, challenging_access(), sink)
        .await
        .expect("bind");
    let handle = tokio::spawn(driver.run());

    let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
    // Send an oversized datagram (dropped), then a valid one (answered).
    client
        .send_to(&vec![b'a'; 4096], local_addr)
        .await
        .expect("send oversized");
    client
        .send_to(&encode_register("34020000001320000001", 5060), local_addr)
        .await
        .expect("send valid");

    let mut buf = vec![0u8; 65535];
    let (len, _source) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
        .await
        .expect("receive within timeout")
        .expect("recv_from");
    match parse_datagram(&buf[..len]) {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 401),
        _ => panic!("expected response"),
    }

    handle.abort();
}

#[tokio::test]
async fn bind_without_addresses_is_rejected() {
    let config = DriverConfig::empty();
    let sink: Arc<dyn EventSink<Gb28181Event>> = Arc::new(NoOpEventSink);
    let result = Gb28181UdpDriver::bind(config, challenging_access(), sink).await;
    assert!(matches!(result, Err(DriverError::NoBindAddress)));
}

#[tokio::test]
async fn ipv6_udp_bind_reports_v6_local_addr() {
    let config = DriverConfig::new("[::1]:0".parse().unwrap());
    let sink: Arc<dyn EventSink<Gb28181Event>> = Arc::new(NoOpEventSink);
    let (driver, local): (_, SocketAddr) =
        match Gb28181UdpDriver::bind(config, challenging_access(), sink).await {
            Ok(pair) => pair,
            // Some CI sandboxes disable IPv6 loopback; skip rather than fail.
            Err(DriverError::Bind(_)) => return,
            Err(e) => panic!("unexpected bind error: {e}"),
        };
    assert!(local.is_ipv6());
    drop(build_register_request("34020000001320000001", 5060));
    drop(driver);
}

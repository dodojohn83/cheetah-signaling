//! TCP framing, connection-limit, cancellation and shutdown contract tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use cheetah_gb28181_core::{SipMessage, SipParser, SipParserConfig};
use cheetah_gb28181_driver_tokio::Gb28181UdpDriver;
use cheetah_gb28181_driver_tokio::config::DriverConfig;
use cheetah_gb28181_driver_tokio::sink::{EventSink, NoOpEventSink};
use cheetah_gb28181_module::Gb28181Event;
use common::{challenging_access, encode_register};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

fn noop_sink() -> Arc<dyn EventSink<Gb28181Event>> {
    Arc::new(NoOpEventSink)
}

/// Binds a TCP-only driver and returns its local address plus the run handle.
async fn spawn_tcp_driver(
    config: DriverConfig,
    cancel: CancellationToken,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let (driver, primary) = Gb28181UdpDriver::bind(config, challenging_access(), noop_sink())
        .await
        .expect("bind");
    let addr = driver.tcp_addrs()[0];
    assert_eq!(primary, addr);
    let handle = tokio::spawn(async move {
        driver.run_with_cancellation(cancel).await.expect("run");
    });
    (addr, handle)
}

/// Reads a single framed SIP response using a persistent stream parser so that
/// bytes for a following message are not discarded between reads.
async fn read_response(stream: &mut TcpStream, parser: &mut SipParser) -> SipMessage {
    let mut buf = vec![0u8; 4096];
    loop {
        if let Some(result) = parser.pop_message() {
            return result.expect("valid framed response");
        }
        let n = timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("read within timeout")
            .expect("read ok");
        assert!(n > 0, "connection closed before a full response arrived");
        parser.feed(&buf[..n]).expect("feed parser");
    }
}

fn stream_parser() -> SipParser {
    SipParser::new(SipParserConfig::default())
}

fn assert_401(message: SipMessage) {
    match message {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 401),
        _ => panic!("expected response"),
    }
}

#[tokio::test]
async fn tcp_register_gets_401_challenge() {
    let config = DriverConfig::empty().with_tcp_bind("127.0.0.1:0".parse().unwrap());
    let cancel = CancellationToken::new();
    let (addr, handle) = spawn_tcp_driver(config, cancel.clone()).await;

    let mut client = TcpStream::connect(addr).await.expect("connect");
    client
        .write_all(&encode_register("34020000001320000001", 5060))
        .await
        .expect("write");
    assert_401(read_response(&mut client, &mut stream_parser()).await);

    cancel.cancel();
    handle.await.expect("driver joined");
}

#[tokio::test]
async fn tcp_handles_byte_by_byte_delivery() {
    let config = DriverConfig::empty().with_tcp_bind("127.0.0.1:0".parse().unwrap());
    let cancel = CancellationToken::new();
    let (addr, handle) = spawn_tcp_driver(config, cancel.clone()).await;

    let mut client = TcpStream::connect(addr).await.expect("connect");
    let bytes = encode_register("34020000001320000001", 5060);
    // Deliver one byte at a time to exercise incremental framing (half packets).
    for chunk in bytes.chunks(1) {
        client.write_all(chunk).await.expect("write");
        client.flush().await.expect("flush");
    }
    assert_401(read_response(&mut client, &mut stream_parser()).await);

    cancel.cancel();
    handle.await.expect("driver joined");
}

#[tokio::test]
async fn tcp_handles_coalesced_messages() {
    let config = DriverConfig::empty().with_tcp_bind("127.0.0.1:0".parse().unwrap());
    let cancel = CancellationToken::new();
    let (addr, handle) = spawn_tcp_driver(config, cancel.clone()).await;

    let mut client = TcpStream::connect(addr).await.expect("connect");
    let mut coalesced = encode_register("34020000001320000001", 5060);
    coalesced.extend_from_slice(&encode_register("34020000001320000002", 5060));
    client.write_all(&coalesced).await.expect("write");

    // Two requests in one write must yield two framed responses.
    let mut parser = stream_parser();
    assert_401(read_response(&mut client, &mut parser).await);
    assert_401(read_response(&mut client, &mut parser).await);

    cancel.cancel();
    handle.await.expect("driver joined");
}

#[tokio::test]
async fn tcp_connection_limit_rejects_excess() {
    let config = DriverConfig::empty()
        .with_tcp_bind("127.0.0.1:0".parse().unwrap())
        .with_max_tcp_connections(1);
    let cancel = CancellationToken::new();
    let (addr, handle) = spawn_tcp_driver(config, cancel.clone()).await;

    // First connection holds the only permit.
    let mut first = TcpStream::connect(addr).await.expect("connect first");
    first
        .write_all(&encode_register("34020000001320000001", 5060))
        .await
        .expect("write");
    assert_401(read_response(&mut first, &mut stream_parser()).await);

    // Second connection is accepted at TCP level then immediately closed.
    let mut second = TcpStream::connect(addr).await.expect("connect second");
    let mut buf = vec![0u8; 64];
    let n = timeout(Duration::from_secs(2), second.read(&mut buf))
        .await
        .expect("read within timeout")
        .expect("read ok");
    assert_eq!(n, 0, "excess connection should be closed with EOF");

    cancel.cancel();
    handle.await.expect("driver joined");
}

#[tokio::test]
async fn cancellation_completes_run_and_stops_accepting() {
    let config = DriverConfig::empty()
        .with_tcp_bind("127.0.0.1:0".parse().unwrap())
        .with_shutdown_drain(Duration::from_secs(2));
    let cancel = CancellationToken::new();
    let (addr, handle) = spawn_tcp_driver(config, cancel.clone()).await;

    let mut client = TcpStream::connect(addr).await.expect("connect");
    client
        .write_all(&encode_register("34020000001320000001", 5060))
        .await
        .expect("write");
    assert_401(read_response(&mut client, &mut stream_parser()).await);

    cancel.cancel();
    // The run future must return promptly after cancellation (within drain).
    timeout(Duration::from_secs(5), handle)
        .await
        .expect("run completes after cancel")
        .expect("driver joined");

    // After shutdown, new connections no longer get answered.
    if let Ok(mut late) = TcpStream::connect(addr).await {
        let mut buf = vec![0u8; 64];
        let _ = late
            .write_all(&encode_register("34020000001320000001", 5060))
            .await;
        let read = timeout(Duration::from_secs(1), late.read(&mut buf)).await;
        if let Ok(Ok(n)) = read {
            assert_eq!(n, 0, "listener should be closed");
        }
    }
}

#[tokio::test]
async fn mixed_udp_and_tcp_bind_reports_both() {
    let config = DriverConfig::new("127.0.0.1:0".parse().unwrap())
        .with_tcp_bind("127.0.0.1:0".parse().unwrap());
    let (driver, _primary) = Gb28181UdpDriver::bind(config, challenging_access(), noop_sink())
        .await
        .expect("bind");
    assert_eq!(driver.udp_addrs().len(), 1);
    assert_eq!(driver.tcp_addrs().len(), 1);
}

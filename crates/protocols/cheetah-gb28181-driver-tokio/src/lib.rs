//! Tokio-based UDP driver for the GB28181 access module.
//!
//! The driver binds a UDP socket, parses incoming SIP datagrams, forwards them
//! to any Sans-I/O [`GbAccessMachine`], and sends any produced SIP responses
//! back to the source address. Domain events are emitted through a generic
//! [`EventSink`] so the caller can forward them to a message bus or log them.

#![warn(missing_docs)]

pub mod config;
pub mod error;
pub mod sink;

use cheetah_gb28181_core::{AccessInput, AccessOutput, GbAccessMachine, SipParser, encode_message};
use config::DriverConfig;
use error::DriverError;
use sink::EventSink;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::MissedTickBehavior;
use tracing::{debug, trace, warn};

/// A UDP transport driver that executes a GB28181 access state machine.
pub struct Gb28181UdpDriver<M: GbAccessMachine> {
    socket: Arc<UdpSocket>,
    access: Mutex<M>,
    sink: Arc<dyn EventSink<M::Event>>,
    parser_config: cheetah_gb28181_core::SipParserConfig,
    max_datagram_size: usize,
    started_at: Instant,
}

impl<M: GbAccessMachine> std::fmt::Debug for Gb28181UdpDriver<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181UdpDriver")
            .field("local_addr", &self.socket.local_addr())
            .field("parser_config", &self.parser_config)
            .field("max_datagram_size", &self.max_datagram_size)
            .finish_non_exhaustive()
    }
}

impl<M: GbAccessMachine + Send + 'static> Gb28181UdpDriver<M> {
    /// Creates a driver bound to `config.bind_addr` with the supplied state
    /// machine and event sink.
    ///
    /// Returns the driver and the local address it actually bound to (useful
    /// when `config.bind_addr` uses port `0`).
    pub async fn bind(
        config: DriverConfig,
        access: M,
        sink: Arc<dyn EventSink<M::Event>>,
    ) -> Result<(Self, SocketAddr), DriverError> {
        let socket = UdpSocket::bind(config.bind_addr)
            .await
            .map_err(DriverError::Bind)?;
        let local_addr = socket.local_addr().map_err(DriverError::Bind)?;

        Ok((
            Self {
                socket: Arc::new(socket),
                access: Mutex::new(access),
                sink,
                parser_config: config.parser_config,
                max_datagram_size: config.max_datagram_size,
                started_at: Instant::now(),
            },
            local_addr,
        ))
    }

    /// Runs the driver loop until the socket is closed.
    ///
    /// Incoming datagrams are parsed and forwarded to [`GbAccessMachine`]. A
    /// periodic tick (once per second) is also forwarded so that registration
    /// expiry and heartbeat timeouts are processed.
    pub async fn run(self) -> Result<(), DriverError> {
        let mut buf = vec![0u8; self.max_datagram_size];
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                result = self.socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, source)) => {
                            let data = &buf[..len];
                            if let Err(e) = self.handle_datagram(data, source).await {
                                warn!(error = %e, %source, "failed to handle SIP datagram");
                            }
                        }
                        Err(e) => return Err(DriverError::Io(e)),
                    }
                }
                _ = interval.tick() => {
                    if let Err(e) = self.handle_tick().await {
                        warn!(error = %e, "failed to process access tick");
                    }
                }
            }
        }
    }

    async fn handle_datagram(&self, data: &[u8], source: SocketAddr) -> Result<(), DriverError> {
        let message =
            SipParser::parse_datagram(data, self.parser_config).map_err(DriverError::Parse)?;
        trace!(%source, "received SIP datagram");

        let input = AccessInput {
            source,
            now: self.now_seconds(),
            message,
        };

        let outputs = {
            let mut access = self.access.lock().map_err(|_| DriverError::AccessLock)?;
            access
                .process(input)
                .map_err(|e| DriverError::Access(Box::new(e)))?
        };

        self.dispatch_outputs(outputs, Some(source)).await
    }

    async fn handle_tick(&self) -> Result<(), DriverError> {
        let now = self.now_seconds();
        let outputs = {
            let mut access = self.access.lock().map_err(|_| DriverError::AccessLock)?;
            access
                .tick(now)
                .map_err(|e| DriverError::Access(Box::new(e)))?
        };

        self.dispatch_outputs(outputs, None).await
    }

    async fn dispatch_outputs(
        &self,
        outputs: Vec<AccessOutput<M::Event>>,
        response_target: Option<SocketAddr>,
    ) -> Result<(), DriverError> {
        for output in outputs {
            match output {
                AccessOutput::SendResponse(response) => {
                    if let Some(target) = response_target {
                        let bytes = encode_message(&response);
                        self.socket
                            .send_to(&bytes, target)
                            .await
                            .map_err(DriverError::Io)?;
                        debug!(%target, "sent SIP response");
                    } else {
                        warn!("dropping SIP response produced by tick with no response target");
                    }
                }
                AccessOutput::EmitEvent(event) => {
                    self.sink.emit(event);
                }
            }
        }

        Ok(())
    }

    fn now_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use cheetah_gb28181_core::{
        HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
    };
    use cheetah_gb28181_module::{
        AuthPolicy, CredentialError, CredentialProvider, DeviceId, Gb28181Access,
        Gb28181DomainConfig, Gb28181Event,
    };
    use secrecy::SecretString;
    use sink::NoOpEventSink;
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::net::UdpSocket;
    use tokio::time::timeout;

    fn test_domain_config(policy: AuthPolicy) -> Gb28181DomainConfig {
        Gb28181DomainConfig::new(
            "test-domain",
            "test.realm",
            std::iter::repeat_n(b'a', 32).collect::<Vec<u8>>(),
        )
        .expect("valid test config")
        .with_auth_policy(policy)
    }

    fn test_credential_provider(
        passwords: HashMap<String, SecretString>,
    ) -> impl CredentialProvider + 'static {
        move |id: &DeviceId| -> Result<Option<SecretString>, CredentialError> {
            Ok(passwords.get(&id.to_string()).cloned())
        }
    }

    fn build_register_request(device_id: &str, port: u16) -> SipMessage {
        let uri = SipUri::parse(format!("sip:{device_id}@127.0.0.1")).expect("valid uri");
        let from_uri = SipUri::parse(format!("sip:{device_id}@127.0.0.1")).expect("valid uri");
        let contact_uri =
            SipUri::parse(format!("sip:{device_id}@127.0.0.1:{port}")).expect("valid uri");

        let mut headers = SipHeaders::new();
        headers.append(
            HeaderName::Via,
            HeaderValue::via("UDP", "127.0.0.1", port, "z9hG4bKregister").expect("valid via"),
        );
        headers.append(
            HeaderName::From,
            HeaderValue::from_uri(&from_uri, "fromtag").expect("valid from"),
        );
        headers.append(HeaderName::To, HeaderValue::to_uri(&from_uri));
        headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
        headers.append(HeaderName::CSeq, HeaderValue::cseq(1, Method::Register));
        headers.append(HeaderName::Contact, HeaderValue::contact_uri(&contact_uri));
        headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
        headers.append(HeaderName::Expires, HeaderValue::new("3600"));

        SipMessage::Request {
            line: RequestLine::new(Method::Register, uri),
            headers,
            body: Vec::new(),
        }
    }

    #[tokio::test]
    async fn register_without_credentials_gets_401_challenge() {
        let config = DriverConfig::new("127.0.0.1:0".parse().unwrap());
        let domain = test_domain_config(AuthPolicy::Required);
        let provider = test_credential_provider(HashMap::new());
        let access = Gb28181Access::new(domain, provider).expect("valid access");
        let sink: Arc<dyn EventSink<Gb28181Event>> = Arc::new(NoOpEventSink);
        let (driver, local_addr) = Gb28181UdpDriver::bind(config, access, sink)
            .await
            .expect("bind");

        let handle = tokio::spawn(driver.run());

        let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
        let request = build_register_request("34020000001320000001", 5060);
        let bytes = encode_message(&request);
        client.send_to(&bytes, local_addr).await.expect("send");

        let mut buf = vec![0u8; 65535];
        let (len, _source) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
            .await
            .expect("receive within timeout")
            .expect("recv_from");

        let response = SipParser::parse_datagram(
            &buf[..len],
            cheetah_gb28181_core::SipParserConfig::default(),
        )
        .expect("parse response");

        if let SipMessage::Response { line, headers, .. } = response {
            assert_eq!(line.code, 401);
            assert!(headers.get(&HeaderName::WwwAuthenticate).is_some());
        } else {
            panic!("expected response");
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
        let provider = test_credential_provider(passwords);
        let access = Gb28181Access::new(domain, provider).expect("valid access");
        let sink: Arc<dyn EventSink<Gb28181Event>> = Arc::new(NoOpEventSink);
        let (driver, local_addr) = Gb28181UdpDriver::bind(config, access, sink)
            .await
            .expect("bind");

        let handle = tokio::spawn(driver.run());

        let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
        let request = build_register_request("34020000001320000001", 5060);
        let bytes = encode_message(&request);
        client.send_to(&bytes, local_addr).await.expect("send");

        let mut buf = vec![0u8; 65535];
        let (len, _source) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
            .await
            .expect("receive within timeout")
            .expect("recv_from");

        let response = SipParser::parse_datagram(
            &buf[..len],
            cheetah_gb28181_core::SipParserConfig::default(),
        )
        .expect("parse response");

        if let SipMessage::Response { line, .. } = response {
            assert_eq!(line.code, 200);
        } else {
            panic!("expected response");
        }

        handle.abort();
    }
}

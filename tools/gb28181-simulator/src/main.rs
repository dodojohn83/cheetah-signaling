//! Multi-device GB28181 simulator for registration, heartbeat, catalog and
//! synthetic media sessions.
//!
//! Each device binds a UDP port, performs SIP registration with digest
//! authentication, answers catalog queries, sends keepalives/alarms and responds
//! to INVITE playback requests.  A seeded RNG provides stable device IDs and
//! deterministic fault/malformed behavior for regression tests.

use cheetah_gb28181_core::{
    DigestChallenge, DigestClient, HeaderName, HeaderValue, Method, RequestLine, SipHeaders,
    SipMessage, SipParser, SipParserConfig, SipUri, StatusLine, encode_message,
};
use cheetah_gb28181_module::xml::{
    CatalogItem, build_alarm_notify, build_catalog_response, build_keepalive, parse_catalog_query,
};
use clap::Parser;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use secrecy::SecretString;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{self, Instant};
use tracing::{info, warn};

/// Runtime configuration.
#[derive(Clone, Parser)]
#[command(name = "gb28181-simulator")]
struct Config {
    /// Platform SIP registrar address.
    #[arg(long, default_value = "127.0.0.1:5060")]
    server: SocketAddr,

    /// Number of simulated devices.
    #[arg(long, default_value = "1")]
    count: usize,

    /// Base device identifier (prefix).  The index is appended for each device.
    #[arg(long, default_value = "34020000001320000001")]
    base_device_id: String,

    /// Random seed for stable device IDs and behavior.
    #[arg(long, default_value = "0")]
    seed: u64,

    /// Shared SIP password for all devices.
    #[arg(long, default_value = "12345678")]
    password: String,

    /// Vendor profile: generic, dahua, hikvision.
    #[arg(long, default_value = "generic")]
    profile: String,

    /// Fraction of outgoing packets to send malformed (0.0..1.0).
    #[arg(long, default_value = "0.0")]
    malformed_rate: f64,

    /// Fraction of incoming commands to intentionally fail (0.0..1.0).
    #[arg(long, default_value = "0.0")]
    failure_rate: f64,

    /// Minimum seconds before first keepalive after registration.
    #[arg(long, default_value = "5")]
    keepalive_start_delay_sec: u64,

    /// Send an unsolicited alarm NOTIFY every N keepalive cycles (0 disables).
    #[arg(long, default_value = "0")]
    alarm_every_n_heartbeats: usize,

    /// Disconnect the socket for a random duration every N keepalive cycles (0 disables).
    #[arg(long, default_value = "0")]
    disconnect_every_n_heartbeats: usize,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("server", &self.server)
            .field("count", &self.count)
            .field("base_device_id", &self.base_device_id)
            .field("seed", &self.seed)
            .field("password", &"[REDACTED]")
            .field("profile", &self.profile)
            .field("malformed_rate", &self.malformed_rate)
            .field("failure_rate", &self.failure_rate)
            .field("keepalive_start_delay_sec", &self.keepalive_start_delay_sec)
            .field("alarm_every_n_heartbeats", &self.alarm_every_n_heartbeats)
            .field(
                "disconnect_every_n_heartbeats",
                &self.disconnect_every_n_heartbeats,
            )
            .finish()
    }
}

impl Config {
    fn profile(&self) -> Profile {
        Profile::resolve(&self.profile)
    }

    fn device_id(&self, index: usize) -> String {
        let base = self.base_device_id.trim();
        let suffix = format!("{:04}", index);
        let max_base = 20usize.saturating_sub(suffix.chars().count());
        let base_chars: String = base.chars().take(max_base).collect();
        format!(
            "{}{}",
            base_chars,
            suffix.chars().take(20).collect::<String>()
        )
    }

    fn effective_failure_rate(&self) -> f64 {
        if self.failure_rate.is_finite() {
            self.failure_rate.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    fn effective_malformed_rate(&self) -> f64 {
        if self.malformed_rate.is_finite() {
            self.malformed_rate.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    fn local_addr(&self, _index: usize) -> String {
        "0.0.0.0:0".to_string()
    }
}

#[derive(Clone, Debug)]
struct Profile {
    heartbeat_sec: u64,
    catalog_item_count: u32,
    manufacturer: String,
    model: String,
    register_timeout_sec: u64,
}

impl Profile {
    fn resolve(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "dahua" => Self {
                heartbeat_sec: 15,
                catalog_item_count: 2,
                manufacturer: "Dahua".to_string(),
                model: "IPC-SIM-001".to_string(),
                register_timeout_sec: 30,
            },
            "hikvision" => Self {
                heartbeat_sec: 20,
                catalog_item_count: 1,
                manufacturer: "Hikvision".to_string(),
                model: "DS-2CDSIM-001".to_string(),
                register_timeout_sec: 30,
            },
            _ => Self {
                heartbeat_sec: 30,
                catalog_item_count: 1,
                manufacturer: "Cheetah".to_string(),
                model: "SIM-GB28181".to_string(),
                register_timeout_sec: 30,
            },
        }
    }
}

#[derive(thiserror::Error, Debug)]
enum SimError {
    #[error("SIP error: {0}")]
    Sip(#[from] cheetah_gb28181_core::SipError),
    #[error("XML error: {0}")]
    Xml(#[from] cheetah_gb28181_module::AccessError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("registration timed out")]
    RegisterTimeout,
    #[error("registration failed: {0}")]
    RegisterFailed(String),
}

struct DeviceState {
    cseq: u32,
    registered: bool,
    registering: bool,
    auth_challenge: Option<DigestChallenge>,
    rng: StdRng,
    heartbeat_count: usize,
    call_id: String,
    from_tag: String,
    local_port: u16,
    last_register_attempt: Option<Instant>,
}

struct DeviceRuntime {
    config: Config,
    profile: Profile,
    index: usize,
    device_id: String,
    password: SecretString,
    state: Arc<Mutex<DeviceState>>,
    socket: Arc<UdpSocket>,
    server: SocketAddr,
    server_domain: String,
}

#[tokio::main]
async fn main() -> Result<(), SimError> {
    let config = Config::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut handles = Vec::new();

    for index in 0..config.count {
        let device_id = config.device_id(index);
        let password = SecretString::new(config.password.clone().into());
        let mut rng = StdRng::seed_from_u64(config.seed.wrapping_add(index as u64));
        let call_id = format!("call-{}-{}", device_id, config.seed);
        let from_tag = format!("tag-{}-{}", device_id, rng.r#gen::<u64>());

        let socket = UdpSocket::bind(config.local_addr(index)).await?;
        let local_port = socket.local_addr()?.port();
        let state = Arc::new(Mutex::new(DeviceState {
            cseq: 1,
            registered: false,
            registering: false,
            auth_challenge: None,
            rng,
            heartbeat_count: 0,
            call_id,
            from_tag,
            local_port,
            last_register_attempt: None,
        }));
        let runtime = DeviceRuntime {
            config: config.clone(),
            profile: config.profile(),
            index,
            device_id,
            password,
            state,
            socket: Arc::new(socket),
            server: config.server,
            server_domain: config.server.ip().to_string(),
        };
        handles.push(tokio::spawn(run_device(runtime)));
    }

    tokio::signal::ctrl_c().await?;
    futures::future::join_all(handles).await;
    Ok(())
}

async fn run_device(runtime: DeviceRuntime) -> Result<(), SimError> {
    let device_id = runtime.device_id.clone();
    info!(%device_id, local_port = runtime.socket.local_addr()?.port(), "starting GB28181 simulator device");

    if runtime.config.failure_rate != runtime.config.effective_failure_rate() {
        warn!(
            failure_rate = %runtime.config.failure_rate,
            effective = %runtime.config.effective_failure_rate(),
            "failure_rate out of range, clamping to [0, 1]"
        );
    }
    if runtime.config.malformed_rate != runtime.config.effective_malformed_rate() {
        warn!(
            malformed_rate = %runtime.config.malformed_rate,
            effective = %runtime.config.effective_malformed_rate(),
            "malformed_rate out of range, clamping to [0, 1]"
        );
    }

    // Register with retries until success or shutdown.
    register_device(&runtime).await?;

    let start = Instant::now() + Duration::from_secs(runtime.config.keepalive_start_delay_sec);
    let mut heartbeat =
        time::interval_at(start, Duration::from_secs(runtime.profile.heartbeat_sec));

    let mut buf = vec![0u8; 8192];
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if let Err(e) = on_heartbeat(&runtime).await {
                    warn!(error = %e, "heartbeat failed");
                }
            }
            received = runtime.socket.recv_from(&mut buf) => {
                let (len, _peer) = received?;
                let data = &buf[..len];
                if let Err(e) = handle_incoming(&runtime, data).await {
                    warn!(error = %e, "failed to handle incoming datagram");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!(%device_id, "GB28181 simulator device stopped");
                break;
            }
        }
    }
    Ok(())
}

async fn register_device(runtime: &DeviceRuntime) -> Result<(), SimError> {
    let deadline = Instant::now() + Duration::from_secs(runtime.profile.register_timeout_sec);
    {
        let mut state = runtime.state.lock().await;
        state.registering = true;
        state.last_register_attempt = Some(Instant::now());
    }

    // Initial REGISTER (no auth).
    send_register(runtime, false).await?;

    let mut buf = vec![0u8; 8192];
    loop {
        let timeout = time::sleep_until(deadline.min(Instant::now() + Duration::from_secs(1)));
        tokio::select! {
            _ = timeout => {
                {
                    let state = runtime.state.lock().await;
                    if state.registered {
                        return Ok(());
                    }
                }
                if Instant::now() >= deadline {
                    let mut state = runtime.state.lock().await;
                    state.registering = false;
                    return Err(SimError::RegisterTimeout);
                }
            }
            received = runtime.socket.recv_from(&mut buf) => {
                let (len, _peer) = received?;
                let data = &buf[..len];
                match handle_incoming(runtime, data).await {
                    Ok(()) => {
                        let state = runtime.state.lock().await;
                        if state.registered {
                            return Ok(());
                        }
                    }
                    Err(e) => warn!(error = %e, "register response failed"),
                }
            }
        }
    }
}

async fn on_heartbeat(runtime: &DeviceRuntime) -> Result<(), SimError> {
    let cycle = {
        let mut state = runtime.state.lock().await;
        if state.registered {
            state.heartbeat_count += 1;
        }
        state.heartbeat_count
    };

    if !runtime.state.lock().await.registered {
        return register_device(runtime).await;
    }

    if should_trigger(runtime, runtime.config.disconnect_every_n_heartbeats, cycle) {
        let reconnect_ms = {
            let mut state = runtime.state.lock().await;
            state.rng.gen_range(500..5000)
        };
        info!(device_id = %runtime.device_id, "simulated disconnect");
        {
            let mut state = runtime.state.lock().await;
            state.registered = false;
            state.registering = true;
        }
        time::sleep(Duration::from_millis(reconnect_ms)).await;
        register_device(runtime).await?;
        return Ok(());
    }

    let xml = build_keepalive(&current_cseq_string(runtime).await, &runtime.device_id, "OK")?;
    let msg = build_message(
        runtime,
        Method::Message,
        xml.as_bytes().to_vec(),
        "application/MANSCDP+xml",
    )
    .await?;
    send_message(runtime, &msg).await?;

    if should_trigger(runtime, runtime.config.alarm_every_n_heartbeats, cycle) {
        let alarm_xml = build_alarm_notify(
            &current_cseq_string(runtime).await,
            &runtime.device_id,
            Some("1"),
            Some("1"),
            Some(" motion"),
            None,
            None,
        )?;
        let alarm = build_message(
            runtime,
            Method::Message,
            alarm_xml.as_bytes().to_vec(),
            "application/MANSCDP+xml",
        )
        .await?;
        send_message(runtime, &alarm).await?;
    }

    Ok(())
}

fn should_trigger(_runtime: &DeviceRuntime, every: usize, cycle: usize) -> bool {
    every > 0 && cycle.is_multiple_of(every)
}

async fn handle_incoming(runtime: &DeviceRuntime, data: &[u8]) -> Result<(), SimError> {
    if data.is_empty() {
        return Ok(());
    }
    let msg = SipParser::parse_datagram(data, SipParserConfig::default())?;

    match msg {
        SipMessage::Response {
            line,
            headers,
            body,
        } => handle_response(runtime, line.code, &headers, &body).await,
        SipMessage::Request {
            line,
            headers,
            body,
        } => handle_request(runtime, &line.method, &headers, &body).await,
    }
}

async fn handle_response(
    runtime: &DeviceRuntime,
    code: u16,
    headers: &SipHeaders,
    _body: &[u8],
) -> Result<(), SimError> {
    if code == 401 {
        let www = headers.get(&HeaderName::WwwAuthenticate);
        if let Some(value) = www {
            let challenge = DigestChallenge::parse(value.as_str())
                .map_err(|e| SimError::RegisterFailed(e.to_string()))?;
            {
                let mut state = runtime.state.lock().await;
                state.auth_challenge = Some(challenge);
            }
            send_register(runtime, true).await?;
            return Ok(());
        }
    }

    if (200..300).contains(&code) {
        let mut state = runtime.state.lock().await;
        if state.registering {
            state.registering = false;
            state.registered = true;
            info!(device_id = %runtime.device_id, "registered");
        }
    } else if code >= 300 {
        let reason = headers
            .get(&HeaderName::Other("Reason".to_string()))
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();
        warn!(code, %reason, "SIP error response");
    }
    Ok(())
}

async fn handle_request(
    runtime: &DeviceRuntime,
    method: &Method,
    headers: &SipHeaders,
    body: &[u8],
) -> Result<(), SimError> {
    match method {
        Method::Message => handle_message_request(runtime, headers, body).await,
        Method::Invite => handle_invite(runtime, headers).await,
        Method::Bye | Method::Cancel => {
            let response = build_response(runtime, 200, "OK", headers, &[]).await?;
            send_message(runtime, &response).await?;
            Ok(())
        }
        _ => {
            let response = build_response(runtime, 405, "Method Not Allowed", headers, &[]).await?;
            send_message(runtime, &response).await?;
            Ok(())
        }
    }
}

async fn handle_message_request(
    runtime: &DeviceRuntime,
    headers: &SipHeaders,
    body: &[u8],
) -> Result<(), SimError> {
    if runtime.state.lock().await.rng.r#gen::<f64>() < runtime.config.effective_failure_rate() {
        warn!("injected failure: ignoring MESSAGE");
        return Ok(());
    }

    let response = build_response(runtime, 200, "OK", headers, &[]).await?;
    send_message(runtime, &response).await?;

    if let Ok(query) = parse_catalog_query(body)
        && query.device_id == runtime.device_id
    {
        send_catalog_response(runtime, &query.sn).await?;
    }

    Ok(())
}

async fn handle_invite(runtime: &DeviceRuntime, headers: &SipHeaders) -> Result<(), SimError> {
    let trying = build_response(runtime, 100, "Trying", headers, &[]).await?;
    send_message(runtime, &trying).await?;

    let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=Simulated\r\nt=0 0\r\nm=video 10000 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n".as_bytes();
    let mut ok = build_response(runtime, 200, "OK", headers, sdp).await?;
    ok.headers_mut()
        .append(HeaderName::ContentType, HeaderValue::new("application/sdp"));
    send_message(runtime, &ok).await?;
    Ok(())
}

async fn send_register(runtime: &DeviceRuntime, with_auth: bool) -> Result<(), SimError> {
    let (cseq, call_id, from_tag, local_port) = {
        let state = runtime.state.lock().await;
        (
            state.cseq,
            state.call_id.clone(),
            state.from_tag.clone(),
            state.local_port,
        )
    };

    let server_uri = SipUri::parse(format!("sip:{}", runtime.server))?;
    let from_uri = SipUri::parse(format!(
        "sip:{}@{}",
        runtime.device_id, runtime.server_domain
    ))?;
    let to_uri = SipUri::parse(format!(
        "sip:{}@{}",
        runtime.device_id, runtime.server_domain
    ))?;
    let contact_uri = SipUri::parse(format!(
        "sip:{}@127.0.0.1:{}",
        runtime.device_id, local_port
    ))?;

    let mut headers = SipHeaders::new();
    let branch = format!("z9hG4bKreg-{}-{}", runtime.index, cseq);
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "127.0.0.1", local_port, &branch)?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&from_uri, &from_tag)?,
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&to_uri));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.clone()));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, Method::Register));
    headers.append(HeaderName::Contact, HeaderValue::contact_uri(&contact_uri));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(
        HeaderName::UserAgent,
        HeaderValue::new("CheetahGBSim/0.1.0"),
    );
    headers.append(HeaderName::Expires, HeaderValue::new("3600"));

    if with_auth {
        let challenge = {
            let state = runtime.state.lock().await;
            state.auth_challenge.clone()
        };
        if let Some(challenge) = challenge {
            let cnonce = DigestClient::derive_cnonce(&runtime.password, &call_id)
                .map_err(|e| SimError::RegisterFailed(e.to_string()))?;
            let mut client = DigestClient::new().allow_md5(true);
            let response = client
                .authorize(
                    &runtime.device_id,
                    &runtime.password,
                    "REGISTER",
                    &server_uri.encode(),
                    &challenge,
                    &cnonce,
                )
                .map_err(|e| SimError::RegisterFailed(e.to_string()))?;
            headers.append(
                HeaderName::Authorization,
                HeaderValue::new(response.to_header_value()),
            );
        } else {
            warn!("requested auth but no challenge stored");
        }
    }

    let msg = SipMessage::Request {
        line: RequestLine::new(Method::Register, server_uri),
        headers,
        body: Vec::new(),
    };

    {
        let mut state = runtime.state.lock().await;
        state.cseq += 1;
        state.last_register_attempt = Some(Instant::now());
    }
    send_message(runtime, &msg).await
}

async fn send_catalog_response(runtime: &DeviceRuntime, sn: &str) -> Result<(), SimError> {
    let mut items = Vec::new();
    for i in 0..runtime.profile.catalog_item_count {
        let channel_id = format!("{}-ch{}", runtime.device_id, i);
        items.push(CatalogItem {
            device_id: channel_id,
            name: Some(format!("Channel {}", i)),
            manufacturer: Some(runtime.profile.manufacturer.clone()),
            model: Some(runtime.profile.model.clone()),
            owner: None,
            civil_code: None,
            block: None,
            address: None,
            parental: Some("0".to_string()),
            parent_id: Some(runtime.device_id.clone()),
            safety_way: None,
            register_way: Some("1".to_string()),
            cert_num: None,
            certifiable: None,
            err_code: Some("0".to_string()),
            end_time: None,
            secrecy: Some("0".to_string()),
            ip_address: None,
            port: None,
            status: Some("ON".to_string()),
            longitude: None,
            latitude: None,
        });
    }
    let xml = build_catalog_response(sn, &runtime.device_id, items.len() as u32, &items)?;
    let msg = build_message(
        runtime,
        Method::Message,
        xml.as_bytes().to_vec(),
        "application/MANSCDP+xml",
    )
    .await?;
    send_message(runtime, &msg).await
}

async fn build_message(
    runtime: &DeviceRuntime,
    method: Method,
    body: Vec<u8>,
    content_type: &str,
) -> Result<SipMessage, SimError> {
    let (cseq, call_id, from_tag, local_port) = {
        let mut state = runtime.state.lock().await;
        let cseq = state.cseq;
        state.cseq += 1;
        (
            cseq,
            state.call_id.clone(),
            state.from_tag.clone(),
            state.local_port,
        )
    };

    let server_uri = SipUri::parse(format!("sip:{}", runtime.server))?;
    let from_uri = SipUri::parse(format!(
        "sip:{}@{}",
        runtime.device_id, runtime.server_domain
    ))?;
    let to_uri = SipUri::parse(format!("sip:{}", runtime.server_domain))?;
    let contact_uri = SipUri::parse(format!(
        "sip:{}@127.0.0.1:{}",
        runtime.device_id, local_port
    ))?;

    let mut headers = SipHeaders::new();
    let branch = format!("z9hG4bKmsg-{}-{}", runtime.index, cseq);
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "127.0.0.1", local_port, &branch)?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&from_uri, &from_tag)?,
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&to_uri));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.clone()));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, method.clone()));
    headers.append(HeaderName::Contact, HeaderValue::contact_uri(&contact_uri));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(
        HeaderName::UserAgent,
        HeaderValue::new("CheetahGBSim/0.1.0"),
    );
    headers.append(HeaderName::ContentType, HeaderValue::new(content_type));

    Ok(SipMessage::Request {
        line: RequestLine::new(method, server_uri),
        headers,
        body,
    })
}

async fn build_response(
    runtime: &DeviceRuntime,
    code: u16,
    reason: &str,
    request_headers: &SipHeaders,
    body: &[u8],
) -> Result<SipMessage, SimError> {
    let local_port = runtime.state.lock().await.local_port;
    let mut headers = SipHeaders::new();
    for (name, value) in request_headers.iter() {
        if matches!(
            name,
            HeaderName::Via
                | HeaderName::From
                | HeaderName::To
                | HeaderName::CallId
                | HeaderName::CSeq
        ) {
            headers.append(name.clone(), value.clone());
        }
    }
    if let Some(via) = request_headers.get(&HeaderName::Via) {
        let contact_uri = SipUri::parse(format!(
            "sip:{}@127.0.0.1:{}",
            runtime.device_id, local_port
        ))?;
        headers.append(HeaderName::Contact, HeaderValue::contact_uri(&contact_uri));
        let _ = via;
    }

    Ok(SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: body.to_vec(),
    })
}

async fn send_message(runtime: &DeviceRuntime, msg: &SipMessage) -> Result<(), SimError> {
    let bytes = encode_message(msg);
    if runtime.state.lock().await.rng.r#gen::<f64>() < runtime.config.effective_malformed_rate() {
        let mut corrupted = bytes;
        if !corrupted.is_empty() {
            corrupted[0] = corrupted[0].wrapping_add(1);
        }
        runtime.socket.send_to(&corrupted, runtime.server).await?;
    } else {
        runtime.socket.send_to(&bytes, runtime.server).await?;
    }
    Ok(())
}

async fn current_cseq_string(runtime: &DeviceRuntime) -> String {
    let state = runtime.state.lock().await;
    state.cseq.to_string()
}

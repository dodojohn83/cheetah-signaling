//! ONVIF device simulator for testing Cheetah Signaling's ONVIF driver.
//!
//! Provides HTTP SOAP endpoints for Device, Media, PTZ and Events services,
//! WS-Security username-token authentication, clock drift, synthetic faults,
//! and a UDP WS-Discovery `Probe` responder.

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::Engine;
use clap::Parser;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use secrecy::{ExposeSecret, SecretString};
use sha1::{Digest, Sha1};
use std::io::Cursor;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use time::{Duration as TimeDuration, OffsetDateTime, format_description::well_known::Iso8601};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{info, warn};

const NS_SOAP: &str = "http://www.w3.org/2003/05/soap-envelope";
const NS_DEVICE: &str = "http://www.onvif.org/ver10/device/wsdl";
const NS_MEDIA: &str = "http://www.onvif.org/ver10/media/wsdl";
const NS_PTZ: &str = "http://www.onvif.org/ver20/ptz/wsdl";
const NS_EVENTS: &str = "http://www.onvif.org/ver10/events/wsdl";
const NS_SCHEMA: &str = "http://www.onvif.org/ver10/schema";
const NS_DISCOVERY: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery";
const NS_WSA: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing";
const NS_DN: &str = "http://www.onvif.org/ver10/network/wsdl";
const XML_DECLARATION: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
const MAX_XML_DEPTH: usize = 64;
const MAX_XML_NODES: usize = 4096;
const MAX_XML_TEXT_BYTES: usize = 8192;

/// CLI configuration.
#[derive(Clone, Debug, Parser)]
#[command(name = "onvif-simulator")]
struct Config {
    /// HTTP bind address for ONVIF SOAP services.
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: SocketAddr,

    /// UDP bind address for WS-Discovery.
    #[arg(long, default_value = "0.0.0.0:3702")]
    discovery_bind: SocketAddr,

    /// Authorized ONVIF username.
    #[arg(long, default_value = "admin")]
    user: String,

    /// Authorized ONVIF password.
    #[arg(long, default_value = "admin")]
    password: SecretString,

    /// Device manufacturer.
    #[arg(long, default_value = "Cheetah")]
    manufacturer: String,

    /// Device model.
    #[arg(long, default_value = "SIM-ONVIF")]
    model: String,

    /// Firmware version.
    #[arg(long, default_value = "0.1.0")]
    firmware: String,

    /// Serial number.
    #[arg(long, default_value = "SIM00000001")]
    serial: String,

    /// Synthetic hardware ID.
    #[arg(long, default_value = "hw-sim-0")]
    hardware_id: String,

    /// Host advertised in WS-Discovery XAddrs (defaults to bind address).
    #[arg(long)]
    xaddr_host: Option<String>,

    /// Fraction of requests to answer with a SOAP Fault (0.0..1.0).
    #[arg(long, default_value = "0.0")]
    fault_rate: f64,

    /// Simulated clock drift applied to GetSystemDateAndTime (milliseconds).
    #[arg(long, default_value = "0")]
    clock_drift_ms: i64,

    /// Stable seed for deterministic fault decisions.
    #[arg(long, default_value = "0")]
    seed: u64,
}

struct AppState {
    config: Config,
    rng: Mutex<StdRng>,
}

#[derive(Clone, Default, Debug)]
struct WsseToken {
    username: Option<String>,
    password: Option<String>,
    password_type: Option<String>,
    password_digest: Option<String>,
    nonce: Option<String>,
    created: Option<String>,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let state = Arc::new(AppState {
        config: config.clone(),
        rng: Mutex::new(StdRng::seed_from_u64(config.seed)),
    });

    let app = Router::new()
        .route("/onvif/device_service", post(onvif_service))
        .route("/onvif/media_service", post(onvif_service))
        .route("/onvif/ptz_service", post(onvif_service))
        .route("/onvif/events_service", post(onvif_service))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(state.clone());

    let listener = match tokio::net::TcpListener::bind(config.bind).await {
        Ok(l) => l,
        Err(e) => {
            warn!(error = %e, addr = %config.bind, "failed to bind ONVIF HTTP listener");
            return;
        }
    };
    info!(addr = %config.bind, "ONVIF HTTP simulator listening");

    let http = axum::serve(listener, app);
    let discovery = run_discovery(config.clone());

    tokio::select! {
        result = http => { if let Err(e) = result { warn!(error = %e, "HTTP server error"); } }
        result = discovery => { if let Err(e) = result { warn!(error = %e, "discovery error"); } }
        _ = tokio::signal::ctrl_c() => {},
    }
}

async fn onvif_service(State(state): State<Arc<AppState>>, body: Bytes) -> impl IntoResponse {
    let (action, token) = parse_envelope(&body);
    let action = action.unwrap_or_default();

    const ANONYMOUS_ACTIONS: &[&str] = &["GetSystemDateAndTime", "GetCapabilities"];
    if !ANONYMOUS_ACTIONS.contains(&action.as_str())
        && let Err(e) = validate_wsse(&state.config, &token)
    {
        return soap_fault(StatusCode::UNAUTHORIZED, "Sender", "NotAuthorized", &e);
    }

    if should_fault(&state).await {
        return soap_fault(
            StatusCode::OK,
            "Receiver",
            "ActionNotSupported",
            "injected fault",
        );
    }

    match build_action_response(&state.config, &action).await {
        Ok(xml) => soap_response(xml),
        Err(msg) => soap_fault(StatusCode::OK, "Receiver", "ActionNotSupported", &msg),
    }
}

async fn build_action_response(config: &Config, action: &str) -> Result<String, String> {
    let mut out = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut out);

    let namespaces: &[(&str, &str)] = match action {
        "GetDeviceInformation"
        | "GetSystemDateAndTime"
        | "GetCapabilities"
        | "GetDiscoveryMode" => &[("tds", NS_DEVICE), ("tt", NS_SCHEMA)],
        "GetProfiles" | "GetStreamUri" => &[("trt", NS_MEDIA), ("tt", NS_SCHEMA)],
        "GetNodes" | "GetConfigurations" => &[("tptz", NS_PTZ), ("tt", NS_SCHEMA)],
        "CreatePullPointSubscription" | "PullMessages" | "GetEventProperties" => &[
            ("tev", NS_EVENTS),
            ("tns1", "http://www.onvif.org/ver10/topics"),
            ("wsa", NS_WSA),
            ("wsnt", "http://docs.oasis-open.org/wsn/b-2"),
        ],
        _ => &[],
    };

    write_envelope(&mut writer, namespaces, |w| {
        let response_name = format!("{}Response", action);
        let mut resp = BytesStart::new(response_name.as_str());
        for (prefix, ns) in namespaces {
            let key = format!("xmlns:{prefix}");
            resp.push_attribute((key.as_str(), *ns));
        }
        if let Some((_, default_ns)) = namespaces.first() {
            resp.push_attribute(("xmlns", *default_ns));
        }
        w.write_event(Event::Start(resp))?;

        match action {
            "GetDeviceInformation" => write_device_info(w, config),
            "GetSystemDateAndTime" => write_date_time(w, config),
            "GetCapabilities" => write_capabilities(w),
            "GetDiscoveryMode" => write_element(w, "DiscoveryMode", "Discoverable"),
            "GetProfiles" => write_profiles(w),
            "GetStreamUri" => write_stream_uri(w, config),
            "GetNodes" => write_ptz_nodes(w),
            "GetConfigurations" => write_ptz_configurations(w),
            "CreatePullPointSubscription" => write_subscription(w, config),
            "PullMessages" => write_pull_messages(w),
            "GetEventProperties" => write_event_properties(w),
            _ => Err(std::io::Error::other("unsupported action")),
        }?;

        w.write_event(Event::End(BytesEnd::new(response_name.as_str())))
    })
    .map_err(|e| e.to_string())?;

    String::from_utf8(out.into_inner()).map_err(|e| e.to_string())
}

fn write_envelope<W: std::io::Write>(
    writer: &mut Writer<W>,
    namespaces: &[(&str, &str)],
    body: impl FnOnce(&mut Writer<W>) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut env = BytesStart::new("soap:Envelope");
    env.push_attribute(("xmlns:soap", NS_SOAP));
    for (prefix, ns) in namespaces {
        let key = format!("xmlns:{prefix}");
        env.push_attribute((key.as_str(), *ns));
    }
    writer.write_event(Event::Start(env))?;
    writer.write_event(Event::Start(BytesStart::new("soap:Header")))?;
    writer.write_event(Event::End(BytesEnd::new("soap:Header")))?;
    writer.write_event(Event::Start(BytesStart::new("soap:Body")))?;
    body(writer)?;
    writer.write_event(Event::End(BytesEnd::new("soap:Body")))?;
    writer.write_event(Event::End(BytesEnd::new("soap:Envelope")))
}

fn write_device_info<W: std::io::Write>(
    writer: &mut Writer<W>,
    config: &Config,
) -> std::io::Result<()> {
    write_element(writer, "tds:Manufacturer", &config.manufacturer)?;
    write_element(writer, "tds:Model", &config.model)?;
    write_element(writer, "tds:FirmwareVersion", &config.firmware)?;
    write_element(writer, "tds:SerialNumber", &config.serial)?;
    write_element(writer, "tds:HardwareId", &config.hardware_id)
}

fn current_datetime(drift_ms: i64) -> OffsetDateTime {
    let drift = TimeDuration::milliseconds(drift_ms);
    OffsetDateTime::now_utc()
        .checked_add(drift)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn write_date_time<W: std::io::Write>(
    writer: &mut Writer<W>,
    config: &Config,
) -> std::io::Result<()> {
    let dt = current_datetime(config.clock_drift_ms);

    let mut dt_type = BytesStart::new("tt:DateTime");
    dt_type.push_attribute(("TimeType", "UTC"));
    writer.write_event(Event::Start(dt_type))?;
    writer.write_event(Event::Start(BytesStart::new("tt:Time")))?;
    write_element(writer, "tt:Hour", &format!("{:02}", dt.hour()))?;
    write_element(writer, "tt:Minute", &format!("{:02}", dt.minute()))?;
    write_element(writer, "tt:Second", &format!("{:02}", dt.second()))?;
    writer.write_event(Event::End(BytesEnd::new("tt:Time")))?;
    writer.write_event(Event::Start(BytesStart::new("tt:Date")))?;
    write_element(writer, "tt:Year", &dt.year().to_string())?;
    write_element(writer, "tt:Month", &format!("{:02}", dt.month() as u8))?;
    write_element(writer, "tt:Day", &format!("{:02}", dt.day()))?;
    writer.write_event(Event::End(BytesEnd::new("tt:Date")))?;
    writer.write_event(Event::End(BytesEnd::new("tt:DateTime")))
}

fn write_capabilities<W: std::io::Write>(writer: &mut Writer<W>) -> std::io::Result<()> {
    let mut caps = BytesStart::new("tds:Capabilities");
    caps.push_attribute(("xmlns:tt", NS_SCHEMA));
    writer.write_event(Event::Start(caps))?;

    writer.write_event(Event::Start(BytesStart::new("tt:Device")))?;
    write_element(writer, "tt:XAddr", "/onvif/device_service")?;
    writer.write_event(Event::Empty(BytesStart::new("tt:Network")))?;
    writer.write_event(Event::Empty(BytesStart::new("tt:System")))?;
    writer.write_event(Event::End(BytesEnd::new("tt:Device")))?;

    writer.write_event(Event::Start(BytesStart::new("tt:Media")))?;
    write_element(writer, "tt:XAddr", "/onvif/media_service")?;
    writer.write_event(Event::Empty(BytesStart::new("tt:StreamingCapabilities")))?;
    writer.write_event(Event::End(BytesEnd::new("tt:Media")))?;

    writer.write_event(Event::Start(BytesStart::new("tt:PTZ")))?;
    write_element(writer, "tt:XAddr", "/onvif/ptz_service")?;
    writer.write_event(Event::End(BytesEnd::new("tt:PTZ")))?;

    writer.write_event(Event::Start(BytesStart::new("tt:Events")))?;
    write_element(writer, "tt:XAddr", "/onvif/events_service")?;
    writer.write_event(Event::End(BytesEnd::new("tt:Events")))?;

    writer.write_event(Event::End(BytesEnd::new("tds:Capabilities")))
}

fn write_profiles<W: std::io::Write>(writer: &mut Writer<W>) -> std::io::Result<()> {
    writer.write_event(Event::Start(BytesStart::new("trt:Profiles")))?;
    write_element(writer, "tt:Name", "sim_profile_0")?;
    writer.write_event(Event::Start(BytesStart::new("tt:VideoSourceConfiguration")))?;
    write_element(writer, "tt:Name", "sim_vsrc")?;
    write_element(writer, "tt:SourceToken", "vsrc_0")?;
    writer.write_event(Event::End(BytesEnd::new("tt:VideoSourceConfiguration")))?;
    writer.write_event(Event::End(BytesEnd::new("trt:Profiles")))
}

fn write_stream_uri<W: std::io::Write>(
    writer: &mut Writer<W>,
    config: &Config,
) -> std::io::Result<()> {
    writer.write_event(Event::Start(BytesStart::new("trt:MediaUri")))?;
    write_element(
        writer,
        "tt:Uri",
        &format!("rtsp://{}:{}/stream0", config.bind.ip(), config.bind.port()),
    )?;
    write_element(writer, "tt:InvalidAfterConnect", "false")?;
    write_element(writer, "tt:InvalidAfterReboot", "false")?;
    write_element(writer, "tt:Timeout", "PT0S")?;
    writer.write_event(Event::End(BytesEnd::new("trt:MediaUri")))
}

fn write_ptz_nodes<W: std::io::Write>(writer: &mut Writer<W>) -> std::io::Result<()> {
    writer.write_event(Event::Start(BytesStart::new("tptz:PTZNode")))?;
    write_element(writer, "tt:Name", "sim_ptz_node")?;
    writer.write_event(Event::Empty(BytesStart::new("tt:SupportedPTZSpaces")))?;
    writer.write_event(Event::End(BytesEnd::new("tptz:PTZNode")))
}

fn write_ptz_configurations<W: std::io::Write>(writer: &mut Writer<W>) -> std::io::Result<()> {
    writer.write_event(Event::Start(BytesStart::new("tptz:PTZConfiguration")))?;
    write_element(writer, "tt:Name", "sim_ptz_config")?;
    write_element(writer, "tt:NodeToken", "ptz_0")?;
    writer.write_event(Event::End(BytesEnd::new("tptz:PTZConfiguration")))
}

fn write_subscription<W: std::io::Write>(
    writer: &mut Writer<W>,
    config: &Config,
) -> std::io::Result<()> {
    writer.write_event(Event::Start(BytesStart::new("tev:SubscriptionReference")))?;
    writer.write_event(Event::Start(BytesStart::new("wsa:Address")))?;
    writer.write_event(Event::Text(BytesText::new(&format!(
        "http://{}:{}/onvif/events_sub",
        config.bind.ip(),
        config.bind.port()
    ))))?;
    writer.write_event(Event::End(BytesEnd::new("wsa:Address")))?;
    writer.write_event(Event::End(BytesEnd::new("tev:SubscriptionReference")))
}

fn write_pull_messages<W: std::io::Write>(writer: &mut Writer<W>) -> std::io::Result<()> {
    write_element(writer, "tev:CurrentTime", &iso_now())?;
    write_element(writer, "tev:TerminationTime", &iso_now())?;
    writer.write_event(Event::Start(BytesStart::new("tev:NotificationMessage")))?;
    write_element(writer, "wsnt:Topic", "tns1:Device/tns1:Trigger")?;
    writer.write_event(Event::End(BytesEnd::new("tev:NotificationMessage")))
}

fn write_event_properties<W: std::io::Write>(writer: &mut Writer<W>) -> std::io::Result<()> {
    write_element(
        writer,
        "tev:TopicNamespaceLocation",
        "http://www.onvif.org/ver10/topics",
    )?;
    writer.write_event(Event::Start(BytesStart::new("tev:FixedTopicSet")))?;
    write_element(
        writer,
        "wsnt:TopicExpressionDialect",
        "http://www.onvif.org/ver10/topics",
    )?;
    writer.write_event(Event::End(BytesEnd::new("tev:FixedTopicSet")))
}

fn write_element<W: std::io::Write>(
    writer: &mut Writer<W>,
    name: &str,
    text: &str,
) -> std::io::Result<()> {
    writer.write_event(Event::Start(BytesStart::new(name)))?;
    writer.write_event(Event::Text(BytesText::new(text)))?;
    writer.write_event(Event::End(BytesEnd::new(name)))
}

fn parse_envelope(body: &[u8]) -> (Option<String>, WsseToken) {
    let mut reader = Reader::from_reader(body);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<String> = Vec::new();
    let mut in_body = false;
    let mut in_token = false;
    let mut capture: Option<String> = None;
    let mut current = String::new();
    let mut action: Option<String> = None;
    let mut token = WsseToken::default();
    let mut node_count = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if stack.len() >= MAX_XML_DEPTH {
                    warn!("SOAP envelope exceeded maximum XML depth");
                    break;
                }
                node_count += 1;
                if node_count > MAX_XML_NODES {
                    warn!("SOAP envelope exceeded maximum XML nodes");
                    break;
                }
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).into_owned();
                if name == "Body" || name.ends_with(":Body") {
                    in_body = true;
                }
                if name == "UsernameToken" || name.ends_with(":UsernameToken") {
                    in_token = true;
                }
                if in_body && action.is_none() && name != "Body" {
                    action = Some(name.clone());
                }
                let is_password = name == "Password" || name.ends_with(":Password");
                if in_token
                    && (matches!(
                        name.as_str(),
                        "Username" | "Password" | "Nonce" | "Created" | "PasswordDigest"
                    ) || name.ends_with(":Username")
                        || name.ends_with(":Password")
                        || name.ends_with(":Nonce")
                        || name.ends_with(":Created")
                        || name.ends_with(":PasswordDigest"))
                {
                    capture = Some(name.clone());
                    current.clear();
                    if is_password {
                        token.password_type = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.local_name().as_ref() == b"Type")
                            .map(|a| String::from_utf8_lossy(&a.value).into_owned());
                    }
                }
                stack.push(name);
            }
            Ok(Event::Empty(e)) => {
                node_count += 1;
                if node_count > MAX_XML_NODES {
                    warn!("SOAP envelope exceeded maximum XML nodes");
                    break;
                }
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).into_owned();
                if in_body && action.is_none() && name != "Body" {
                    action = Some(name.clone());
                }
                let _ = name;
            }
            Ok(Event::Text(e)) => {
                if capture.is_some()
                    && let Ok(text) = e.decode()
                {
                    if current.len() + text.len() > MAX_XML_TEXT_BYTES {
                        warn!("SOAP text node exceeded maximum length");
                        break;
                    }
                    current.push_str(text.as_ref());
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).into_owned();
                if Some(name.clone()) == capture {
                    let local = if let Some(pos) = name.rfind(':') {
                        &name[pos + 1..]
                    } else {
                        name.as_str()
                    };
                    match local {
                        "Username" => token.username = Some(current.clone()),
                        "Password" => {
                            if token
                                .password_type
                                .as_deref()
                                .is_some_and(|t| t.ends_with("#PasswordDigest"))
                            {
                                token.password_digest = Some(current.clone());
                            } else {
                                token.password = Some(current.clone());
                            }
                        }
                        "Nonce" => token.nonce = Some(current.clone()),
                        "Created" => token.created = Some(current.clone()),
                        "PasswordDigest" => token.password_digest = Some(current.clone()),
                        _ => {}
                    }
                    capture = None;
                    current.clear();
                }
                if name == "UsernameToken" || name.ends_with(":UsernameToken") {
                    in_token = false;
                }
                if name == "Body" || name.ends_with(":Body") {
                    in_body = false;
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                warn!(error = %e, "failed to parse SOAP envelope");
                break;
            }
            _ => {}
        }
    }

    (action, token)
}

fn validate_wsse(config: &Config, token: &WsseToken) -> Result<(), String> {
    let user = token.username.as_deref().unwrap_or("");
    if user != config.user {
        return Err("unknown username".to_string());
    }

    if let Some(digest) = &token.password_digest {
        let nonce = token
            .nonce
            .as_deref()
            .ok_or_else(|| "missing nonce".to_string())?;
        let created = token
            .created
            .as_deref()
            .ok_or_else(|| "missing created".to_string())?;

        let nonce_bytes = base64::engine::general_purpose::STANDARD
            .decode(nonce)
            .map_err(|_| "invalid nonce base64".to_string())?;
        let password = config.password.expose_secret();
        let mut hasher_input =
            Vec::with_capacity(nonce_bytes.len() + created.len() + password.len());
        hasher_input.extend_from_slice(&nonce_bytes);
        hasher_input.extend_from_slice(created.as_bytes());
        hasher_input.extend_from_slice(password.as_bytes());

        let hash = Sha1::digest(&hasher_input);
        let expected = base64::engine::general_purpose::STANDARD.encode(hash);

        if expected != *digest {
            return Err("password digest mismatch".to_string());
        }
    } else if let Some(password) = &token.password {
        if password != config.password.expose_secret() {
            return Err("password mismatch".to_string());
        }
    } else {
        return Err("missing credentials".to_string());
    }

    Ok(())
}

async fn should_fault(state: &Arc<AppState>) -> bool {
    let mut rng = state.rng.lock().await;
    rng.r#gen::<f64>() < state.config.fault_rate
}

fn soap_response(body: String) -> Response<Body> {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/soap+xml; charset=utf-8"),
    );
    response
}

fn soap_fault(status: StatusCode, code: &str, subcode: &str, reason: &str) -> Response<Body> {
    let mut out = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut out);
    if let Err(e) = write_envelope(&mut writer, &[], |w| {
        w.write_event(Event::Start(BytesStart::new("soap:Fault")))?;
        write_element(w, "soap:Code", &format!("soap:{code}"))?;
        write_element(w, "soap:Subcode", &format!("soap:{subcode}"))?;
        write_element(w, "soap:Reason", reason)?;
        write_element(w, "soap:Detail", "simulated ONVIF fault")?;
        w.write_event(Event::End(BytesEnd::new("soap:Fault")))
    }) {
        warn!(error = %e, "failed to build SOAP fault");
    }
    let body = String::from_utf8_lossy(&out.into_inner()).into_owned();
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/soap+xml; charset=utf-8"),
    );
    response
}

fn iso_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .unwrap_or_default()
}

fn discovery_xaddr(bind: SocketAddr, xaddr_host: Option<&str>) -> String {
    let host = if let Some(h) = xaddr_host {
        h.to_string()
    } else {
        url::Host::parse(&bind.ip().to_string())
            .map(|h| h.to_string())
            .unwrap_or_else(|_| bind.ip().to_string())
    };
    let mut xaddr = match url::Url::parse("http://localhost/onvif/device_service") {
        Ok(u) => u,
        Err(_) => return format!("http://{}:{}/onvif/device_service", host, bind.port()),
    };
    xaddr.set_host(Some(&host)).ok();
    xaddr.set_port(Some(bind.port())).ok();
    xaddr.to_string()
}

async fn run_discovery(config: Config) -> std::io::Result<()> {
    let bind = config.discovery_bind;
    let hardware_id = config.hardware_id.clone();
    let socket = UdpSocket::bind(bind).await?;
    if let Ok(addr) = socket.local_addr() {
        info!(addr = %addr, "ONVIF WS-Discovery listener bound");
    }

    let multicast = Ipv4Addr::new(239, 255, 255, 250);
    let _ = socket.join_multicast_v4(multicast, Ipv4Addr::UNSPECIFIED);

    let xaddr = discovery_xaddr(config.bind, config.xaddr_host.as_deref());
    let mut buf = vec![0u8; 8192];
    loop {
        let (len, peer) = socket.recv_from(&mut buf).await?;
        let data = &buf[..len];
        let text = String::from_utf8_lossy(data);
        if text.contains("Probe") {
            let response = probe_match_response(&xaddr, &hardware_id);
            let _ = socket.send_to(response.as_bytes(), peer).await;
        }
    }
}

fn probe_match_response(xaddr: &str, hardware_id: &str) -> String {
    let mut out = XML_DECLARATION.to_vec();
    let mut writer = Writer::new(&mut out);

    let mut env = BytesStart::new("SOAP-ENV:Envelope");
    env.push_attribute(("xmlns:SOAP-ENV", NS_SOAP));
    env.push_attribute(("xmlns:wsa", NS_WSA));
    env.push_attribute(("xmlns:d", NS_DISCOVERY));
    env.push_attribute(("xmlns:dn", NS_DN));
    env.push_attribute(("xmlns:tds", NS_DEVICE));
    let _ = writer.write_event(Event::Start(env));

    let _ = writer.write_event(Event::Start(BytesStart::new("SOAP-ENV:Header")));
    let _ = write_element(&mut writer, "wsa:MessageID", &format!("uuid:{hardware_id}"));
    let _ = write_element(&mut writer, "wsa:RelatesTo", &format!("uuid:{hardware_id}"));
    let _ = write_element(
        &mut writer,
        "wsa:To",
        "http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous",
    );
    let _ = write_element(
        &mut writer,
        "wsa:Action",
        "http://schemas.xmlsoap.org/ws/2005/04/discovery/ProbeMatches",
    );
    let _ = writer.write_event(Event::End(BytesEnd::new("SOAP-ENV:Header")));

    let _ = writer.write_event(Event::Start(BytesStart::new("SOAP-ENV:Body")));
    let _ = writer.write_event(Event::Start(BytesStart::new("d:ProbeMatches")));
    let _ = writer.write_event(Event::Start(BytesStart::new("d:ProbeMatch")));

    let _ = writer.write_event(Event::Start(BytesStart::new("wsa:EndpointReference")));
    let _ = write_element(
        &mut writer,
        "wsa:Address",
        &format!("urn:uuid:{hardware_id}"),
    );
    let _ = writer.write_event(Event::End(BytesEnd::new("wsa:EndpointReference")));

    let _ = write_element(
        &mut writer,
        "d:Types",
        "dn:NetworkVideoTransmitter tds:Device",
    );
    let scopes = format!(
        "onvif://www.onvif.org/type/video_encoder onvif://www.onvif.org/hardware/{hardware_id}"
    );
    let _ = write_element(&mut writer, "d:Scopes", &scopes);
    let _ = write_element(&mut writer, "d:XAddrs", xaddr);
    let _ = write_element(&mut writer, "d:MetadataVersion", "1");

    let _ = writer.write_event(Event::End(BytesEnd::new("d:ProbeMatch")));
    let _ = writer.write_event(Event::End(BytesEnd::new("d:ProbeMatches")));
    let _ = writer.write_event(Event::End(BytesEnd::new("SOAP-ENV:Body")));
    let _ = writer.write_event(Event::End(BytesEnd::new("SOAP-ENV:Envelope")));

    String::from_utf8(out).unwrap_or_default()
}

//! Shared helpers for GB28181 module integration tests.
#![allow(dead_code)]

use cheetah_gb28181_core::{
    DigestChallenge, DigestResponse, HeaderName, HeaderValue, Method, RequestLine, SipHeaders,
    SipMessage, SipUri,
};
use cheetah_gb28181_module::{
    Gb28181Config, Gb28181Module,
    config::{Gb28181ConfigBuilder, InMemoryPasswordLookup},
    module::Gb28181Input,
    output::Gb28181Output,
};
use cheetah_runtime_api::DeviceKey;
use cheetah_signal_types::{DeviceId, TenantId, UtcTimestamp};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

pub const DEVICE_ID: &str = "34020000001320000001";
pub const REALM: &str = "3402000000";
pub const PASSWORD: &str = "testpassword";
pub const SECRET: &str = "this-is-a-very-long-server-secret-used-for-testing-only-do-not-use";

pub fn test_config() -> Arc<Gb28181Config> {
    let mut lookup = InMemoryPasswordLookup::new();
    lookup.insert(DEVICE_ID, PASSWORD);
    let addr = SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 5060));
    Arc::new(
        Gb28181ConfigBuilder::new(TenantId::generate(), REALM, addr)
            .server_secret(SECRET)
            .password_lookup(Arc::new(lookup))
            .build(),
    )
}

pub fn test_module() -> Result<Gb28181Module, Box<dyn std::error::Error>> {
    let config = test_config();
    let key = DeviceKey::new(TenantId::generate(), DeviceId::generate());
    Ok(Gb28181Module::new(key, config)?)
}

pub fn source_addr() -> SocketAddr {
    SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 5061))
}

pub fn now() -> UtcTimestamp {
    UtcTimestamp::default()
}

fn sip_uri(user: &str, host: &str) -> Result<SipUri, Box<dyn std::error::Error>> {
    Ok(SipUri::parse(format!("sip:{user}@{host}"))?)
}

pub fn register_request(
    cseq: u32,
    expires: u32,
    authorization: Option<&str>,
) -> Result<SipMessage, Box<dyn std::error::Error>> {
    let source = source_addr();
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new(format!("SIP/2.0/UDP {source};branch=z9hG4bK{cseq}")),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=test{cseq}")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{cseq} REGISTER")),
    );
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{source}>")),
    );
    headers.append(HeaderName::Expires, HeaderValue::new(expires.to_string()));
    if let Some(auth) = authorization {
        headers.append(HeaderName::Authorization, HeaderValue::new(auth));
    }
    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Register, sip_uri(DEVICE_ID, REALM)?),
        headers,
        body: Vec::new(),
    })
}

pub fn message_request(cseq: u32, body: &[u8]) -> Result<SipMessage, Box<dyn std::error::Error>> {
    let source = source_addr();
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new(format!("SIP/2.0/UDP {source};branch=z9hG4bK{cseq}")),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=test{cseq}")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{cseq} MESSAGE")),
    );
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );
    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Message, sip_uri(DEVICE_ID, REALM)?),
        headers,
        body: body.to_vec(),
    })
}

pub fn authorization_for_challenge(challenge: &DigestChallenge, nc: u64, cnonce: &str) -> String {
    let uri = format!("sip:{DEVICE_ID}@{REALM}");
    let response = DigestResponse::compute_response(
        challenge.algorithm,
        DEVICE_ID,
        REALM,
        PASSWORD,
        &challenge.nonce,
        nc,
        Some(cnonce),
        challenge.qop,
        "REGISTER",
        &uri,
    );
    DigestResponse {
        username: DEVICE_ID.to_string(),
        realm: REALM.to_string(),
        nonce: challenge.nonce.clone(),
        uri,
        response,
        cnonce: Some(cnonce.to_string()),
        nc: Some(nc),
        qop: challenge.qop,
        algorithm: Some(challenge.algorithm),
        opaque: challenge.opaque.clone(),
    }
    .to_header_value()
}

pub fn extract_www_authenticate(outputs: &[Gb28181Output]) -> Option<String> {
    outputs.iter().find_map(|o| match o {
        Gb28181Output::SendMessage {
            message: SipMessage::Response { headers, .. },
            ..
        } => headers
            .get(&HeaderName::WwwAuthenticate)
            .map(|v| v.to_string()),
        _ => None,
    })
}

pub fn challenge_for_module(
    module: &mut Gb28181Module,
    now: UtcTimestamp,
) -> Result<DigestChallenge, Box<dyn std::error::Error>> {
    let request = register_request(1, 3600, None)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now,
    )?;
    let value = extract_www_authenticate(&outputs).ok_or("missing challenge")?;
    Ok(DigestChallenge::parse(&value)?)
}

pub fn register_module() -> Result<Gb28181Module, Box<dyn std::error::Error>> {
    register_module_with_config(test_config())
}

pub fn register_module_with_config(
    config: Arc<Gb28181Config>,
) -> Result<Gb28181Module, Box<dyn std::error::Error>> {
    let mut module = Gb28181Module::new(
        DeviceKey::new(TenantId::generate(), DeviceId::generate()),
        config,
    )?;
    let now = now();
    let challenge = challenge_for_module(&mut module, now)?;
    let auth = authorization_for_challenge(&challenge, 1, "abc123");
    let request = register_request(2, 3600, Some(&auth))?;
    module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now,
    )?;
    Ok(module)
}

pub fn test_config_with_page_size(page_size: u32) -> Arc<Gb28181Config> {
    test_config_with_limits(page_size, 1024, 30, 1024)
}

pub fn test_config_with_limits(
    page_size: u32,
    max_pending: usize,
    pending_timeout: u64,
    max_recent: usize,
) -> Arc<Gb28181Config> {
    let mut lookup = InMemoryPasswordLookup::new();
    lookup.insert(DEVICE_ID, PASSWORD);
    let addr = SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 5060));
    Arc::new(
        Gb28181ConfigBuilder::new(TenantId::generate(), REALM, addr)
            .server_secret(SECRET)
            .password_lookup(Arc::new(lookup))
            .catalog_page_size(page_size)
            .max_pending_commands(max_pending)
            .pending_command_timeout_seconds(pending_timeout)
            .max_recent_messages(max_recent)
            .build(),
    )
}

pub fn is_response_with_code(message: &SipMessage, code: u16) -> bool {
    matches!(message, SipMessage::Response { line, .. } if line.code == code)
}

pub fn count_heartbeats(outputs: &[Gb28181Output]) -> usize {
    outputs
        .iter()
        .filter(|o| matches!(o, Gb28181Output::Heartbeat(..)))
        .count()
}

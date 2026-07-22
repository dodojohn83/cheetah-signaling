//! Shared GB28181 wire-driving helpers for the GB4-SYS system scenarios.
//!
//! These helpers build and drive the real GB28181 access and media Sans-I/O
//! state machines (`Gb28181Access`, `Gb28181Media`) so the system tests can
//! exercise the full REGISTER / keepalive / catalog / alarm / media-negotiation
//! path end to end. No RTP/RTCP/PS/TS/ES media payload is ever produced; media
//! steps only carry the SIP/SDP control handshake.

#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_gb28181_core::{
    GbAccessMachine, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
    StatusLine,
};
use cheetah_gb28181_module::{
    AccessInput, AccessOutput, AuthPolicy, CredentialError, CredentialProvider, DeviceId, DomainId,
    Gb28181Access, Gb28181DomainConfig, Gb28181Event, Gb28181Media, MediaCommand, MediaConfig,
    MediaInput, MediaOutput, MediaTransport,
};
use cheetah_signal_types::{ChannelId, MediaSessionId};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

pub const REALM: &str = "example.com";
pub const DOMAIN: &str = "3402000000";
pub const DEVICE_ID: &str = "34020000001320000001";
pub const PASSWORD: &str = "secret";
pub const SERVER_SECRET: &[u8] = b"server-secret-must-be-32-bytes-long";
pub const SOURCE: &str = "192.168.1.100:5060";

/// Builds an access state machine that challenges REGISTER with SHA-256 digest.
pub fn build_access() -> Gb28181Access<impl CredentialProvider> {
    let config = Gb28181DomainConfig::new(DOMAIN, REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::Required);
    let provider = |device: &DeviceId| -> Result<Option<SecretString>, CredentialError> {
        if device.as_ref() == DEVICE_ID {
            Ok(Some(SecretString::from(PASSWORD)))
        } else {
            Ok(None)
        }
    };
    Gb28181Access::new(config, provider).unwrap()
}

fn hash_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn compute_response(nonce: &str) -> String {
    let a1 = format!("{DEVICE_ID}:{REALM}:{PASSWORD}");
    let ha1 = hash_hex(&a1);
    let a2 = format!("REGISTER:sip:{DEVICE_ID}@{REALM}");
    let ha2 = hash_hex(&a2);
    let a3 = format!("{ha1}:{nonce}:00000001:clientnonce:auth:{ha2}");
    hash_hex(&a3)
}

fn extract_nonce(header: &str) -> String {
    header
        .split(',')
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix("nonce=\"")
                .and_then(|v| v.split('\"').next())
                .map(String::from)
        })
        .expect("nonce in challenge")
}

fn add_authorization(request: &mut SipMessage, nonce: &str) {
    let response = compute_response(nonce);
    let value = format!(
        r##"Digest username="{DEVICE_ID}", realm="{REALM}", nonce="{nonce}", uri="sip:{DEVICE_ID}@{REALM}", response="{response}", cnonce="clientnonce", nc="00000001", qop="auth", algorithm="SHA-256""##
    );
    request
        .headers_mut()
        .append(HeaderName::Authorization, HeaderValue::new(value));
}

fn make_register_request(cseq: u32, expires: u32) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKreg"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("reg-call-id"));
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
    headers.append(HeaderName::UserAgent, HeaderValue::new("IPC"));
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

fn make_message_request(call_id: &str, cseq: u32, body: &[u8]) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKmsg"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{cseq} MESSAGE")),
    );
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
            SipUri::parse(format!("sip:{DEVICE_ID}@{REALM}")).unwrap(),
        ),
        headers,
        body: body.to_vec(),
    }
}

fn find_response(outputs: &[AccessOutput<Gb28181Event>]) -> &SipMessage {
    outputs
        .iter()
        .find_map(|o| match o {
            AccessOutput::SendResponse(m) => Some(m),
            _ => None,
        })
        .expect("a SIP response")
}

/// Returns an iterator over the events emitted in an output batch.
pub fn events(outputs: &[AccessOutput<Gb28181Event>]) -> impl Iterator<Item = &Gb28181Event> + '_ {
    outputs.iter().filter_map(|o| match o {
        AccessOutput::EmitEvent(e) => Some(e),
        _ => None,
    })
}

fn response_code(message: &SipMessage) -> u16 {
    match message {
        SipMessage::Response { line, .. } => line.code,
        SipMessage::Request { .. } => panic!("expected response"),
    }
}

/// Drives a full authenticated REGISTER (401 challenge then 200 OK) and returns
/// the final output batch, which contains the `DeviceRegistered` event.
pub fn register_device(
    access: &mut Gb28181Access<impl CredentialProvider>,
    now: u64,
) -> Vec<AccessOutput<Gb28181Event>> {
    let mut request = make_register_request(1, 3600);
    let challenge = access
        .process(AccessInput {
            source: SOURCE.parse().unwrap(),
            now,
            message: request.clone(),
        })
        .unwrap();
    let www_auth = find_response(&challenge)
        .headers()
        .get(&HeaderName::WwwAuthenticate)
        .expect("WWW-Authenticate")
        .as_str()
        .to_string();
    assert_eq!(response_code(find_response(&challenge)), 401);
    let nonce = extract_nonce(&www_auth);

    add_authorization(&mut request, &nonce);
    let outputs = access
        .process(AccessInput {
            source: SOURCE.parse().unwrap(),
            now,
            message: request,
        })
        .unwrap();
    assert_eq!(response_code(find_response(&outputs)), 200);
    outputs
}

/// Sends a keepalive MESSAGE and returns the emitted outputs.
pub fn keepalive(
    access: &mut Gb28181Access<impl CredentialProvider>,
    now: u64,
) -> Vec<AccessOutput<Gb28181Event>> {
    let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Status>OK</Status>
</Notify>"#;
    let request = make_message_request("keepalive-call", 2, body);
    let outputs = access
        .process(AccessInput {
            source: SOURCE.parse().unwrap(),
            now,
            message: request,
        })
        .unwrap();
    assert_eq!(response_code(find_response(&outputs)), 200);
    outputs
}

/// Sends a catalog response MESSAGE describing a single channel and returns the
/// emitted outputs (which contain a `CatalogReceived` event).
pub fn catalog_response(
    access: &mut Gb28181Access<impl CredentialProvider>,
    now: u64,
) -> Vec<AccessOutput<Gb28181Event>> {
    let body = br#"<?xml version="1.0" encoding="GB2312"?>
<Response>
    <CmdType>Catalog</CmdType>
    <SN>2</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <SumNum>1</SumNum>
    <DeviceList Num="1">
        <Item>
            <DeviceID>34020000001320000002</DeviceID>
            <Name>Camera 1</Name>
            <Status>ON</Status>
        </Item>
    </DeviceList>
</Response>"#;
    let request = make_message_request("catalog-call", 3, body);
    access
        .process(AccessInput {
            source: SOURCE.parse().unwrap(),
            now,
            message: request,
        })
        .unwrap()
}

/// Sends an alarm notification MESSAGE and returns the emitted outputs (which
/// contain an `AlarmReceived` event).
pub fn alarm_notify(
    access: &mut Gb28181Access<impl CredentialProvider>,
    now: u64,
) -> Vec<AccessOutput<Gb28181Event>> {
    let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Alarm</CmdType>
    <SN>4</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <AlarmPriority>1</AlarmPriority>
    <AlarmMethod>2</AlarmMethod>
    <AlarmTime>2024-01-01T00:00:00</AlarmTime>
</Notify>"#;
    let request = make_message_request("alarm-call", 4, body);
    access
        .process(AccessInput {
            source: SOURCE.parse().unwrap(),
            now,
            message: request,
        })
        .unwrap()
}

// ---- Media negotiation (SIP/SDP control only) -----------------------------

/// Builds a media state machine for the local domain.
pub fn build_media() -> Gb28181Media {
    Gb28181Media::new(MediaConfig {
        local_sip_uri: SipUri::parse("sip:server@192.168.1.10:5060").unwrap(),
        max_sessions: 8,
        domain_id: DomainId::new(DOMAIN).unwrap(),
    })
}

/// Builds a `StartLive` command targeting the invited device.
pub fn start_live_command(media_session_id: MediaSessionId, channel_id: ChannelId) -> MediaCommand {
    MediaCommand::StartLive {
        media_session_id,
        channel_id,
        device_id: DeviceId::new(DEVICE_ID).unwrap(),
        target: SipUri::parse("sip:34020000001320000001@192.168.1.20:5060").unwrap(),
        call_id: "media-call-1".to_string(),
        local_tag: "tag-local".to_string(),
        cseq: 1,
        branch: "z9hG4bKmedia".to_string(),
        subject_session: "0200000000".to_string(),
        media_address: "192.168.1.100".to_string(),
        media_port: 5000,
        ssrc: "0200000000".to_string(),
        transport: MediaTransport::TcpPassive,
    }
}

/// Builds a device 200 OK answer to the INVITE, carrying an SDP control body.
pub fn build_invite_ok() -> SipMessage {
    let sdp = "v=0\r\n\
              o=- 0 0 IN IP4 0.0.0.0\r\n\
              s=Play\r\n\
              c=IN IP4 192.168.1.200\r\n\
              t=0 0\r\n\
              m=video 6000 TCP/RTP/AVP 96\r\n\
              a=setup:active\r\n\
              a=connection:new\r\n\
              a=rtpmap:96 PS/90000\r\n\
              a=y:0200000001";
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bKmedia"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("media-call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 INVITE"));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5061>"),
    );
    headers.append(HeaderName::ContentType, HeaderValue::new("application/sdp"));
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(sdp.len().to_string()),
    );
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: sdp.as_bytes().to_vec(),
    }
}

/// Builds the device 200 OK answer to the BYE that stops the session.
pub fn build_bye_ok() -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bKmedia-bye"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("media-call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("2 BYE"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    }
}

/// Drives a full media negotiation: `StartLive` INVITE, device 200 OK (ACK +
/// `MediaSessionStarted`), then stop (BYE + `MediaSessionStopped`). Returns the
/// events observed across the whole handshake.
pub fn negotiate_media(
    media: &mut Gb28181Media,
    media_session_id: MediaSessionId,
    channel_id: ChannelId,
) -> Vec<Gb28181Event> {
    let mut collected = Vec::new();

    let invite = media
        .process(MediaInput::Command(start_live_command(
            media_session_id,
            channel_id,
        )))
        .unwrap();
    assert!(
        invite.iter().any(is_send_message),
        "StartLive must emit an INVITE"
    );

    let answered = media
        .process(MediaInput::Message(build_invite_ok()))
        .unwrap();
    assert!(answered.iter().any(is_send_message), "200 OK must be ACKed");
    collect_events(&answered, &mut collected);

    let stop = media
        .process(MediaInput::Command(MediaCommand::StopMediaSession {
            media_session_id,
        }))
        .unwrap();
    assert!(stop.iter().any(is_send_message), "stop must emit a BYE");

    let stopped = media.process(MediaInput::Message(build_bye_ok())).unwrap();
    collect_events(&stopped, &mut collected);
    collect_events(&stop, &mut collected);

    collected
}

fn is_send_message(output: &MediaOutput) -> bool {
    matches!(output, MediaOutput::SendMessage(_))
}

fn collect_events(outputs: &[MediaOutput], into: &mut Vec<Gb28181Event>) {
    for output in outputs {
        if let MediaOutput::EmitEvent(event) = output {
            into.push(event.clone());
        }
    }
}

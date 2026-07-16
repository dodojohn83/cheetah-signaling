//! SIP/SDP message builders for GB28181 media sessions.

use crate::error::AccessError;
use crate::media::MediaTransport;
use crate::media::session::Session;
use crate::types::DeviceId;
use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri, StatusLine,
    encode_sdp,
};

/// Builds an INVITE request with an SDP offer.
#[allow(clippy::too_many_arguments)]
pub fn build_invite(
    local_uri: &SipUri,
    target: &SipUri,
    call_id: &str,
    local_tag: &str,
    cseq: u32,
    branch: &str,
    device_id: &DeviceId,
    subject_session: &str,
    media_address: &str,
    media_port: u16,
    ssrc: &str,
    transport: MediaTransport,
) -> Result<SipMessage, AccessError> {
    let sdp = build_sdp_offer(media_address, media_port, ssrc, transport)?;
    let body = sdp.into_bytes();

    let mut headers = SipHeaders::new();
    let local_host = local_uri.host();
    let local_port = local_uri.port().unwrap_or(5060);
    headers.append(
        HeaderName::Via,
        HeaderValue::new(format!(
            "SIP/2.0/UDP {local_host}:{local_port};branch={branch}"
        )),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<{}>;tag={local_tag}", local_uri.encode())),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<{}>", target.encode())),
    );
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{cseq} INVITE")));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", local_uri.encode())),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));

    let calling = local_uri.user().unwrap_or("-");
    headers.append(
        HeaderName::Subject,
        HeaderValue::new(format!(
            "{device_id}:{subject_session},{calling}:{device_id}"
        )),
    );
    headers.append(HeaderName::ContentType, HeaderValue::new("application/sdp"));
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Invite, target.clone()),
        headers,
        body,
    })
}

/// Builds an ACK request for a 2xx INVITE response.
pub fn build_ack(
    local_uri: &SipUri,
    session: &Session,
    remote_tag: &str,
    target: &SipUri,
    branch: &str,
) -> SipMessage {
    let mut headers = SipHeaders::new();
    let local_host = local_uri.host();
    let local_port = local_uri.port().unwrap_or(5060);
    headers.append(
        HeaderName::Via,
        HeaderValue::new(format!(
            "SIP/2.0/UDP {local_host}:{local_port};branch={branch}"
        )),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!(
            "<{}>;tag={}",
            local_uri.encode(),
            session.local_tag
        )),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<{}>;tag={}", session.target.encode(), remote_tag)),
    );
    headers.append(
        HeaderName::CallId,
        HeaderValue::new(session.call_id.clone()),
    );
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{} ACK", session.cseq)),
    );
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", local_uri.encode())),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    SipMessage::Request {
        line: RequestLine::new(Method::Ack, target.clone()),
        headers,
        body: Vec::new(),
    }
}

/// Builds a BYE request for an established dialog.
pub fn build_bye(
    local_uri: &SipUri,
    session: &Session,
    cseq: u32,
    branch: &str,
    target: &SipUri,
) -> Result<SipMessage, AccessError> {
    let remote_tag = session
        .remote_tag
        .as_ref()
        .ok_or_else(|| AccessError::Internal("missing remote tag for BYE".to_string()))?;

    let mut headers = SipHeaders::new();
    let local_host = local_uri.host();
    let local_port = local_uri.port().unwrap_or(5060);
    headers.append(
        HeaderName::Via,
        HeaderValue::new(format!(
            "SIP/2.0/UDP {local_host}:{local_port};branch={branch}"
        )),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!(
            "<{}>;tag={}",
            local_uri.encode(),
            session.local_tag
        )),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<{}>;tag={}", session.target.encode(), remote_tag)),
    );
    headers.append(
        HeaderName::CallId,
        HeaderValue::new(session.call_id.clone()),
    );
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{cseq} BYE")));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", local_uri.encode())),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Bye, target.clone()),
        headers,
        body: Vec::new(),
    })
}

/// Builds a `200 OK` response to an in-dialog request.
pub fn build_ok_response(msg: &SipMessage) -> SipMessage {
    let mut headers = SipHeaders::new();
    for (name, value) in msg.headers().iter() {
        if *name == HeaderName::Via
            || *name == HeaderName::From
            || *name == HeaderName::To
            || *name == HeaderName::CallId
            || *name == HeaderName::CSeq
        {
            headers.append(name.clone(), value.clone());
        }
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    }
}

/// Builds the SDP offer for a live or playback session.
pub fn build_sdp_offer(
    media_address: &str,
    media_port: u16,
    ssrc: &str,
    transport: MediaTransport,
) -> Result<String, AccessError> {
    use cheetah_gb28181_core::sdp::{
        RtpMap, SdpAttribute, SdpConnection, SdpConnectionType, SdpDirection, SdpMedia, SdpOrigin,
        SdpSession, SdpSetup, SdpTime,
    };

    let mut media = SdpMedia {
        media_type: "video".to_string(),
        port: media_port,
        port_count: 1,
        proto: transport.proto().to_string(),
        formats: vec!["96".to_string()],
        connection: Some(SdpConnection {
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: media_address.to_string(),
        }),
        attributes: vec![
            SdpAttribute::Direction(SdpDirection::RecvOnly),
            SdpAttribute::Connection(SdpConnectionType::New),
            SdpAttribute::RtpMap(RtpMap {
                pt: "96".to_string(),
                encoding: "PS".to_string(),
                clock: "90000".to_string(),
                params: None,
            }),
            SdpAttribute::Y(ssrc.to_string()),
        ],
        ..Default::default()
    };

    if transport.is_tcp() {
        let setup = match transport {
            MediaTransport::TcpPassive => SdpSetup::Passive,
            MediaTransport::TcpActive => SdpSetup::Active,
            _ => unreachable!(),
        };
        media.attributes.insert(0, SdpAttribute::Setup(setup));
    }

    let session = SdpSession {
        version: "0".to_string(),
        origin: SdpOrigin {
            username: "-".to_string(),
            sess_id: "0".to_string(),
            sess_version: "0".to_string(),
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: media_address.to_string(),
        },
        name: "Play".to_string(),
        connection: Some(SdpConnection {
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: media_address.to_string(),
        }),
        times: vec![SdpTime {
            start: "0".to_string(),
            stop: "0".to_string(),
        }],
        media: vec![media],
        ..Default::default()
    };

    encode_sdp(&session).map_err(|e| AccessError::Internal(e.to_string()))
}

/// Extracts the first URI from a `Contact` header value.
pub fn first_contact_uri(msg: &SipMessage) -> Result<SipUri, super::MediaError> {
    msg.headers()
        .get(&HeaderName::Contact)
        .ok_or_else(|| super::MediaError::MalformedSip("missing Contact header".to_string()))?
        .as_str()
        .split(',')
        .find_map(|token| {
            let token = token.trim();
            let inner = if let Some(start) = token.find('<') {
                let end = token.find('>')?;
                &token[start + 1..end]
            } else {
                token
            };
            SipUri::parse(inner).ok()
        })
        .ok_or_else(|| super::MediaError::MalformedSip("invalid Contact URI".to_string()))
}

/// Extracts a `tag` parameter from a header value.
pub fn tag_from_header(msg: &SipMessage, name: &HeaderName) -> Option<String> {
    msg.headers().get(name).and_then(|v| {
        let value = v.as_str();
        let lower = value.to_ascii_lowercase();
        let start = lower.find(";tag=")? + 5;
        let rest = &value[start..];
        let end = rest
            .find(|c: char| c == ';' || c == '<' || c == '>' || c.is_whitespace())
            .unwrap_or(rest.len());
        Some(rest[..end].trim_matches('"').to_string())
    })
}

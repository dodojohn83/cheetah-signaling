//! SIP/SDP message builders for GB28181 media sessions.

use crate::error::AccessError;
use crate::media::MediaTransport;
use crate::media::session::Session;
use crate::types::DeviceId;
use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri, StatusLine,
    encode_sdp,
};

/// Rejects values that would inject extra SIP header lines.
fn validate_sip_header_token(value: &str) -> Result<(), AccessError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(AccessError::Internal(
            "SIP header token contains forbidden line break".to_string(),
        ));
    }
    Ok(())
}

/// Parameters for an SDP offer.
#[allow(missing_docs)]
#[derive(Clone, Debug)]
pub struct SdpParams {
    /// `s=` session name (`Play`, `Playback`, `Download`, etc.).
    pub session_name: String,
    /// `m=` media type (`video` or `audio`).
    pub media_type: String,
    /// `m=` port.
    pub media_port: u16,
    /// RTP transport token.
    pub transport: MediaTransport,
    /// Media direction attribute.
    pub direction: cheetah_gb28181_core::sdp::SdpDirection,
    /// `t=` time description.
    pub time: cheetah_gb28181_core::sdp::SdpTime,
    /// Optional `a=y:` SSRC for GB28181 video sessions.
    pub ssrc: Option<String>,
    /// `c=` address and origin address.
    pub media_address: String,
    /// Optional explicit `rtpmap`; defaults to PS/90000 when absent.
    pub rtpmap: Option<cheetah_gb28181_core::sdp::RtpMap>,
    /// Extra `a=` attributes appended after the default set.
    pub extra_attrs: Vec<cheetah_gb28181_core::sdp::SdpAttribute>,
}

impl SdpParams {
    /// Returns the default PS/90000 `rtpmap` used for GB28181 video streams.
    pub fn default_video_rtpmap() -> cheetah_gb28181_core::sdp::RtpMap {
        cheetah_gb28181_core::sdp::RtpMap {
            pt: "96".to_string(),
            encoding: "PS".to_string(),
            clock: "90000".to_string(),
            params: None,
        }
    }

    /// Returns a PCMA/8000 `rtpmap` for G.711A audio.
    pub fn pcma_rtpmap() -> cheetah_gb28181_core::sdp::RtpMap {
        cheetah_gb28181_core::sdp::RtpMap {
            pt: "8".to_string(),
            encoding: "PCMA".to_string(),
            clock: "8000".to_string(),
            params: None,
        }
    }

    /// Returns a PCMU/8000 `rtpmap` for G.711U audio.
    pub fn pcmu_rtpmap() -> cheetah_gb28181_core::sdp::RtpMap {
        cheetah_gb28181_core::sdp::RtpMap {
            pt: "0".to_string(),
            encoding: "PCMU".to_string(),
            clock: "8000".to_string(),
            params: None,
        }
    }
}

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
    sdp_params: &SdpParams,
) -> Result<SipMessage, AccessError> {
    let sdp = build_sdp_offer(sdp_params)?;
    let body = sdp.into_bytes();

    validate_sip_header_token(call_id)?;
    validate_sip_header_token(local_tag)?;
    validate_sip_header_token(branch)?;
    validate_sip_header_token(subject_session)?;

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
    remote_tag: Option<&str>,
    target: &SipUri,
    branch: &str,
) -> Result<SipMessage, AccessError> {
    validate_sip_header_token(branch)?;
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
    let to = if let Some(tag) = remote_tag {
        format!("<{}>;tag={tag}", session.target.encode())
    } else {
        format!("<{}>", session.target.encode())
    };
    headers.append(HeaderName::To, HeaderValue::new(to));
    headers.append(
        HeaderName::CallId,
        HeaderValue::new(session.call_id.clone()),
    );
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{} ACK", session.invite_cseq)),
    );
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", local_uri.encode())),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Ack, target.clone()),
        headers,
        body: Vec::new(),
    })
}

/// Builds a BYE request for an established dialog.
pub fn build_bye(
    local_uri: &SipUri,
    session: &Session,
    cseq: u32,
    branch: &str,
    target: &SipUri,
) -> Result<SipMessage, AccessError> {
    validate_sip_header_token(branch)?;
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

/// Builds a CANCEL request for an outstanding INVITE.
pub fn build_cancel(
    local_uri: &SipUri,
    session: &Session,
    cseq: u32,
    branch: &str,
    target: &SipUri,
) -> Result<SipMessage, AccessError> {
    validate_sip_header_token(branch)?;
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
        HeaderValue::new(format!("<{}>", session.target.encode())),
    );
    headers.append(
        HeaderName::CallId,
        HeaderValue::new(session.call_id.clone()),
    );
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{cseq} CANCEL")));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", local_uri.encode())),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Cancel, target.clone()),
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

/// Builds the SDP offer from [`SdpParams`].
pub fn build_sdp_offer(params: &SdpParams) -> Result<String, AccessError> {
    use cheetah_gb28181_core::sdp::{
        SdpAttribute, SdpConnection, SdpConnectionType, SdpMedia, SdpOrigin, SdpSession, SdpSetup,
    };

    let rtpmap = params
        .rtpmap
        .clone()
        .unwrap_or_else(SdpParams::default_video_rtpmap);
    let pt = rtpmap.pt.clone();

    let mut attrs = vec![
        SdpAttribute::Direction(params.direction),
        SdpAttribute::Connection(SdpConnectionType::New),
        SdpAttribute::RtpMap(rtpmap),
    ];
    if let Some(ssrc) = &params.ssrc {
        attrs.push(SdpAttribute::Y(ssrc.clone()));
    }
    attrs.extend(params.extra_attrs.iter().cloned());

    if params.transport.is_tcp() {
        let setup = match params.transport {
            MediaTransport::TcpPassive => SdpSetup::Passive,
            MediaTransport::TcpActive => SdpSetup::Active,
            _ => {
                return Err(AccessError::Internal(
                    "unexpected non-TCP transport in TCP branch".to_string(),
                ));
            }
        };
        attrs.insert(0, SdpAttribute::Setup(setup));
    }

    let media = SdpMedia {
        media_type: params.media_type.clone(),
        port: params.media_port,
        port_count: 1,
        proto: params.transport.proto().to_string(),
        formats: vec![pt],
        connection: Some(SdpConnection {
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: params.media_address.clone(),
        }),
        attributes: attrs,
        ..Default::default()
    };

    let session = SdpSession {
        version: "0".to_string(),
        origin: SdpOrigin {
            username: "-".to_string(),
            sess_id: "0".to_string(),
            sess_version: "0".to_string(),
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: params.media_address.clone(),
        },
        name: params.session_name.clone(),
        connection: Some(SdpConnection {
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: params.media_address.clone(),
        }),
        times: vec![params.time.clone()],
        media: vec![media],
        ..Default::default()
    };

    encode_sdp(&session).map_err(|e| AccessError::Internal(e.to_string()))
}

/// Extracts the first URI from a `Contact` header value.
pub fn first_contact_uri(msg: &SipMessage) -> Result<SipUri, super::MediaError> {
    msg.headers()
        .get(&HeaderName::Contact)
        .ok_or_else(|| super::MediaError::malformed_sip("missing Contact header"))?
        .as_str()
        .split(',')
        .find_map(|token| {
            let token = token.trim();
            let inner = if let Some(start) = token.find('<') {
                let end = token.find('>')?;
                if end <= start {
                    return None;
                }
                &token[start + 1..end]
            } else {
                token
            };
            SipUri::parse(inner).ok()
        })
        .ok_or_else(|| super::MediaError::malformed_sip("invalid Contact URI"))
}

/// Extracts a `tag` parameter from a header value.
pub fn tag_from_header(msg: &SipMessage, name: &HeaderName) -> Option<String> {
    msg.headers().get(name).and_then(|v| {
        let value = v.as_str().trim();
        const NEEDLE: &[u8] = b";tag=";
        for (i, window) in value.as_bytes().windows(NEEDLE.len()).enumerate() {
            if window
                .iter()
                .zip(NEEDLE)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
            {
                let start = i + NEEDLE.len();
                let rest = &value[start..];
                let end = rest
                    .find(|c: char| c == ';' || c == '<' || c == '>' || c.is_whitespace())
                    .unwrap_or(rest.len());
                return Some(rest[..end].trim_matches('"').to_string());
            }
        }
        None
    })
}

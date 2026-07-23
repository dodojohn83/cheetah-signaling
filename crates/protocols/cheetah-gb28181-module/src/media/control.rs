//! GB28181 playback control (MANSRTSP over SIP INFO).

use crate::error::AccessError;
use crate::media::session::Session;
use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
};
use std::fmt::Write;

/// Playback control action delivered via SIP INFO/MANSRTSP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackAction {
    /// Resume or start playback.
    Play,
    /// Pause playback.
    Pause,
    /// Stop playback.
    Teardown,
}

impl PlaybackAction {
    /// MANSRTSP method name for the action.
    pub fn method(self) -> &'static str {
        match self {
            Self::Play => "PLAY",
            Self::Pause => "PAUSE",
            Self::Teardown => "TEARDOWN",
        }
    }

    /// Lower-case MANSRTSP method name, used for branch identifiers.
    pub fn method_lower(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Teardown => "teardown",
        }
    }
}

/// Rejects values that would inject extra MANSRTSP lines.
fn validate_mansrtsp_field(value: &str) -> Result<(), AccessError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(AccessError::Internal(
            "MANSRTSP field contains forbidden line break".to_string(),
        ));
    }
    Ok(())
}

/// Builds a SIP INFO request carrying a `MANSRTSP` body.
#[allow(clippy::too_many_arguments)]
pub fn build_info_mansrtsp(
    local_uri: &SipUri,
    session: &Session,
    cseq: u32,
    branch: &str,
    target: &SipUri,
    action: PlaybackAction,
    scale: Option<f64>,
    range: Option<&str>,
) -> Result<SipMessage, AccessError> {
    validate_mansrtsp_field(branch)?;

    let remote_tag = session
        .remote_tag
        .as_ref()
        .ok_or_else(|| AccessError::Internal("missing remote tag for INFO".to_string()))?;

    let mut body = String::new();
    write!(body, "{} MANSRTSP/1.0\r\n", action.method())
        .map_err(|e| AccessError::Internal(e.to_string()))?;
    write!(body, "CSeq: {cseq}\r\n").map_err(|e| AccessError::Internal(e.to_string()))?;
    if let Some(s) = scale {
        if !s.is_finite() {
            return Err(AccessError::Internal("non-finite Scale value".to_string()));
        }
        let value = format!("{s}");
        validate_mansrtsp_field(&value)?;
        write!(body, "Scale: {value}\r\n").map_err(|e| AccessError::Internal(e.to_string()))?;
    }
    if let Some(r) = range {
        validate_mansrtsp_field(r)?;
        write!(body, "Range: {r}\r\n").map_err(|e| AccessError::Internal(e.to_string()))?;
    }
    let body_bytes = body.into_bytes();

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
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{cseq} INFO")));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", local_uri.encode())),
    );
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("application/MANSRTSP"),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body_bytes.len().to_string()),
    );

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Info, target.clone()),
        headers,
        body: body_bytes,
    })
}

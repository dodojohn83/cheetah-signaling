//! SIP REGISTER builders for upstream GB28181 cascade platforms.

use crate::cascade::{CascadeConfig, CascadeError};
use cheetah_gb28181_core::{
    DigestResponse, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage,
};

/// Builds a REGISTER request for the upstream platform.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_register_request(
    config: &CascadeConfig,
    call_id: &str,
    local_tag: &str,
    cseq: u32,
    branch: &str,
    expires_seconds: u32,
    auth: Option<&DigestResponse>,
) -> Result<SipMessage, CascadeError> {
    validate_token(call_id)?;
    validate_token(local_tag)?;
    validate_token(branch)?;

    let mut headers = SipHeaders::new();
    let local_host = config.local_uri.host();
    let local_port = config.local_uri.port().unwrap_or(5060);
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", local_host, local_port, branch)?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&config.local_uri, local_tag)?,
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&config.local_uri));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, Method::Register));
    headers.append(
        HeaderName::Contact,
        HeaderValue::contact_uri(&config.local_uri),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(
        HeaderName::Expires,
        HeaderValue::new(expires_seconds.to_string()),
    );
    if let Some(ua) = &config.user_agent {
        headers.append(HeaderName::UserAgent, HeaderValue::new(ua.clone()));
    }
    if let Some(auth) = auth {
        headers.append(
            HeaderName::Authorization,
            HeaderValue::new(auth.to_header_value()),
        );
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Register, config.upstream.clone()),
        headers,
        body: Vec::new(),
    })
}

/// Rejects values that would inject extra SIP header lines.
pub(crate) fn validate_token(value: &str) -> Result<(), CascadeError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(CascadeError::Internal(
            "SIP header token contains forbidden line break".to_string(),
        ));
    }
    Ok(())
}

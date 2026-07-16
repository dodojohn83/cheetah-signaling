//! Keepalive MESSAGE builder for upstream GB28181 cascade platforms.

use cheetah_gb28181_core::{
    Body, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage,
};

use crate::cascade::{CascadeConfig, CascadeError};
use crate::xml::build_keepalive;

/// Builds a `MESSAGE` request carrying a GB28181 `Keepalive` XML payload.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_keepalive_message(
    config: &CascadeConfig,
    call_id: &str,
    cseq: u32,
    local_tag: &str,
    branch: &str,
    sn: u32,
    platform_id: &str,
) -> Result<SipMessage, CascadeError> {
    let body_str = build_keepalive(&sn.to_string(), platform_id, "OK")
        .map_err(|e| CascadeError::Internal(format!("failed to encode keepalive XML: {e}")))?;
    let body: Body = body_str.into_bytes();

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
    headers.append(HeaderName::To, HeaderValue::to_uri(&config.upstream));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, Method::Message));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    if let Some(ua) = &config.user_agent {
        headers.append(HeaderName::UserAgent, HeaderValue::new(ua.clone()));
    }
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Message, config.upstream.clone()),
        headers,
        body,
    })
}

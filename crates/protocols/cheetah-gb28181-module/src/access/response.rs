//! GB28181 access module SIP response builders.

use cheetah_gb28181_core::{
    DigestChallenge, HeaderName, HeaderValue, SipHeaders, SipMessage, SipUri, StatusLine,
};

pub(crate) fn build_challenge_response(
    request: &SipMessage,
    challenge: &DigestChallenge,
    tag: String,
) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(
        HeaderName::WwwAuthenticate,
        HeaderValue::new(challenge.to_header_value()),
    );
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(401, "Unauthorized"),
        headers,
        body: Vec::new(),
    }
}

pub(crate) fn build_error_response(
    request: &SipMessage,
    code: u16,
    reason: &str,
    tag: String,
) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: Vec::new(),
    }
}

pub(crate) fn build_rate_limited_response(
    request: &SipMessage,
    retry_after_seconds: u64,
    tag: String,
) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(
        HeaderName::parse("Retry-After"),
        HeaderValue::new(retry_after_seconds.to_string()),
    );
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(429, "Too Many Requests"),
        headers,
        body: Vec::new(),
    }
}

pub(crate) fn build_success_response(
    request: &SipMessage,
    contact: &SipUri,
    expires: u32,
    tag: String,
) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>;expires={}", contact.encode(), expires)),
    );
    headers.append(HeaderName::Expires, HeaderValue::new(expires.to_string()));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    }
}

pub(crate) fn build_message_response(request: &SipMessage, tag: String) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    }
}

fn copy_common_headers(request: &SipMessage) -> SipHeaders {
    let mut headers = SipHeaders::new();
    // Via may appear multiple times (one per proxy hop); copy all of them.
    for value in request.headers().get_all(&HeaderName::Via) {
        headers.append(HeaderName::Via.clone(), value.clone());
    }
    for name in [HeaderName::From, HeaderName::CallId, HeaderName::CSeq] {
        if let Some(value) = request.headers().get(&name) {
            headers.append(name, value.clone());
        }
    }
    headers
}

fn add_or_replace_tag(value: &str, tag: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let without_tag = value
        .split(';')
        .filter(|part| !part.trim().starts_with("tag="))
        .collect::<Vec<_>>()
        .join(";");
    if without_tag.is_empty() {
        format!("tag={tag}")
    } else {
        format!("{without_tag};tag={tag}")
    }
}

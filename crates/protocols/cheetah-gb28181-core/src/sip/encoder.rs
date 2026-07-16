//! SIP encoder producing stable CRLF-framed wire output.

use super::headers::{HeaderName, SipHeaders};
use super::message::{RequestLine, SipMessage, StatusLine};

/// Encodes a SIP message to wire bytes.
pub fn encode_message(message: &SipMessage) -> Vec<u8> {
    let mut out = String::new();
    match message {
        SipMessage::Request {
            line,
            headers,
            body,
        } => {
            encode_request_line(&mut out, line);
            encode_headers(&mut out, headers, body.len());
        }
        SipMessage::Response {
            line,
            headers,
            body,
        } => {
            encode_status_line(&mut out, line);
            encode_headers(&mut out, headers, body.len());
        }
    }
    let mut bytes = out.into_bytes();
    match message {
        SipMessage::Request { body, .. } | SipMessage::Response { body, .. } => {
            bytes.extend_from_slice(body);
        }
    }
    bytes
}

fn encode_request_line(out: &mut String, line: &RequestLine) {
    out.push_str(&line.method.to_string());
    out.push(' ');
    out.push_str(&line.uri.encode());
    out.push(' ');
    out.push_str(&line.version);
    out.push_str("\r\n");
}

fn encode_status_line(out: &mut String, line: &StatusLine) {
    out.push_str(&line.version);
    out.push(' ');
    out.push_str(&line.code.to_string());
    out.push(' ');
    out.push_str(&line.reason);
    out.push_str("\r\n");
}

fn encode_headers(out: &mut String, headers: &SipHeaders, body_len: usize) {
    // Re-emit Content-Length based on actual body bytes unless already present.
    let mut emitted = false;
    for (name, value) in headers.iter() {
        if *name == HeaderName::ContentLength {
            out.push_str("Content-Length: ");
            out.push_str(&body_len.to_string());
            out.push_str("\r\n");
            emitted = true;
        } else {
            out.push_str(name.as_str());
            out.push_str(": ");
            out.push_str(value.as_str());
            out.push_str("\r\n");
        }
    }
    if !emitted {
        out.push_str("Content-Length: ");
        out.push_str(&body_len.to_string());
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
}

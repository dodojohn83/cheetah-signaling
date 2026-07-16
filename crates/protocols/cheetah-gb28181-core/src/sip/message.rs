//! SIP message model: method, start lines, headers and body.

use super::error::{SipError, SipErrorKind};
use super::headers::{HeaderName, SipHeaders};
use super::uri::SipUri;

/// SIP methods required for GB28181.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Method {
    /// REGISTER registration.
    Register,
    /// MESSAGE for catalog/notify.
    Message,
    /// INVITE session establishment.
    Invite,
    /// ACK confirmation.
    Ack,
    /// BYE session teardown.
    Bye,
    /// CANCEL pending request.
    Cancel,
    /// OPTIONS capability query.
    Options,
    /// SUBSCRIBE (reserved).
    Subscribe,
    /// NOTIFY (reserved).
    Notify,
    /// Other method not in the GB28181 subset.
    Other(String),
}

impl Method {
    /// Parses a method from its wire name.
    pub fn parse(s: &str) -> Result<Self, SipError> {
        Ok(match s {
            "REGISTER" => Method::Register,
            "MESSAGE" => Method::Message,
            "INVITE" => Method::Invite,
            "ACK" => Method::Ack,
            "BYE" => Method::Bye,
            "CANCEL" => Method::Cancel,
            "OPTIONS" => Method::Options,
            "SUBSCRIBE" => Method::Subscribe,
            "NOTIFY" => Method::Notify,
            other if other.chars().all(|c| c.is_ascii_uppercase() || c == '-') => {
                Method::Other(other.to_string())
            }
            _ => {
                return Err(SipError::new(
                    SipErrorKind::InvalidStartLine,
                    None,
                    "invalid method token",
                ));
            }
        })
    }
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Method::Register => "REGISTER",
            Method::Message => "MESSAGE",
            Method::Invite => "INVITE",
            Method::Ack => "ACK",
            Method::Bye => "BYE",
            Method::Cancel => "CANCEL",
            Method::Options => "OPTIONS",
            Method::Subscribe => "SUBSCRIBE",
            Method::Notify => "NOTIFY",
            Method::Other(other) => other.as_str(),
        };
        write!(f, "{s}")
    }
}

/// SIP/2.0 version constant.
pub const SIP_2_0: &str = "SIP/2.0";

/// Request line of a SIP request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestLine {
    /// SIP method.
    pub method: Method,
    /// Request-URI.
    pub uri: SipUri,
    /// SIP version, normally `SIP/2.0`.
    pub version: String,
}

impl RequestLine {
    /// Creates a new request line.
    pub fn new(method: Method, uri: SipUri) -> Self {
        Self {
            method,
            uri,
            version: SIP_2_0.to_string(),
        }
    }
}

/// Status line of a SIP response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusLine {
    /// SIP version.
    pub version: String,
    /// Numeric status code.
    pub code: u16,
    /// Reason phrase.
    pub reason: String,
}

impl StatusLine {
    /// Creates a new status line.
    pub fn new(code: u16, reason: impl Into<String>) -> Self {
        Self {
            version: SIP_2_0.to_string(),
            code,
            reason: reason.into(),
        }
    }

    /// Classifies the response category.
    pub fn class(&self) -> ResponseClass {
        match self.code / 100 {
            1 => ResponseClass::Provisional,
            2 => ResponseClass::Success,
            3 => ResponseClass::Redirection,
            4 => ResponseClass::ClientFailure,
            5 => ResponseClass::ServerFailure,
            6 => ResponseClass::GlobalFailure,
            _ => ResponseClass::Unknown,
        }
    }
}

/// Response class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseClass {
    /// 1xx
    Provisional,
    /// 2xx
    Success,
    /// 3xx
    Redirection,
    /// 4xx
    ClientFailure,
    /// 5xx
    ServerFailure,
    /// 6xx
    GlobalFailure,
    /// Unknown or malformed.
    Unknown,
}

/// A parsed SIP message body as raw bytes.
pub type Body = Vec<u8>;

/// A SIP request or response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SipMessage {
    /// SIP request.
    Request {
        /// Request line.
        line: RequestLine,
        /// SIP headers.
        headers: SipHeaders,
        /// Body bytes.
        body: Body,
    },
    /// SIP response.
    Response {
        /// Status line.
        line: StatusLine,
        /// SIP headers.
        headers: SipHeaders,
        /// Body bytes.
        body: Body,
    },
}

impl SipMessage {
    /// Returns the headers of the message.
    pub fn headers(&self) -> &SipHeaders {
        match self {
            SipMessage::Request { headers, .. } => headers,
            SipMessage::Response { headers, .. } => headers,
        }
    }

    /// Returns the mutable headers of the message.
    pub fn headers_mut(&mut self) -> &mut SipHeaders {
        match self {
            SipMessage::Request { headers, .. } => headers,
            SipMessage::Response { headers, .. } => headers,
        }
    }

    /// Returns the body bytes.
    pub fn body(&self) -> &[u8] {
        match self {
            SipMessage::Request { body, .. } => body,
            SipMessage::Response { body, .. } => body,
        }
    }

    /// Returns the `Call-ID` header value, if present.
    pub fn call_id(&self) -> Option<&str> {
        self.headers().get(&HeaderName::CallId).map(|v| v.as_str())
    }

    /// Returns the top `Via` branch parameter, if present.
    pub fn top_branch(&self) -> Option<&str> {
        self.headers()
            .get_all(&HeaderName::Via)
            .next()
            .and_then(|v| branch_value(v.as_str()))
    }

    /// Returns the `CSeq` value as `(number, method)`, if parseable.
    pub fn cseq(&self) -> Option<(u32, Method)> {
        self.headers().get(&HeaderName::CSeq).and_then(|v| {
            let mut parts = v.as_str().splitn(2, char::is_whitespace);
            let num = parts.next()?.parse().ok()?;
            let method = Method::parse(parts.next()?).ok()?;
            Some((num, method))
        })
    }

    /// Returns `Content-Length` header value, if present and valid.
    pub fn content_length(&self) -> Option<usize> {
        self.headers()
            .get(&HeaderName::ContentLength)
            .and_then(|v| v.as_str().trim().parse().ok())
    }
}

fn branch_value(via: &str) -> Option<&str> {
    for token in via.split(';') {
        let token = token.trim();
        if let Some(value) = token.strip_prefix("branch=") {
            return Some(value.trim_matches('"'));
        }
    }
    None
}

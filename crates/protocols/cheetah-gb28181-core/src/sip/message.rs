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
    /// INFO for in-dialog control (e.g., MANSRTSP playback control).
    Info,
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
            "INFO" => Method::Info,
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

    /// Parses a method from its wire name, accepting any case and normalizing
    /// unknown tokens to upper case.
    ///
    /// This is used when the `HeaderNormalization` compatibility capability is
    /// enabled so that a non-uppercase method (e.g. `register`) does not force a
    /// temporary allocation of the whole start-line before validation.
    pub fn parse_normalized(s: &str) -> Result<Self, SipError> {
        if s.eq_ignore_ascii_case("REGISTER") {
            return Ok(Method::Register);
        }
        if s.eq_ignore_ascii_case("MESSAGE") {
            return Ok(Method::Message);
        }
        if s.eq_ignore_ascii_case("INVITE") {
            return Ok(Method::Invite);
        }
        if s.eq_ignore_ascii_case("ACK") {
            return Ok(Method::Ack);
        }
        if s.eq_ignore_ascii_case("BYE") {
            return Ok(Method::Bye);
        }
        if s.eq_ignore_ascii_case("CANCEL") {
            return Ok(Method::Cancel);
        }
        if s.eq_ignore_ascii_case("OPTIONS") {
            return Ok(Method::Options);
        }
        if s.eq_ignore_ascii_case("SUBSCRIBE") {
            return Ok(Method::Subscribe);
        }
        if s.eq_ignore_ascii_case("NOTIFY") {
            return Ok(Method::Notify);
        }
        if s.eq_ignore_ascii_case("INFO") {
            return Ok(Method::Info);
        }
        if s.chars().all(|c| c.is_ascii_alphabetic() || c == '-') && !s.is_empty() {
            return Ok(Method::Other(s.to_ascii_uppercase()));
        }
        Err(SipError::new(
            SipErrorKind::InvalidStartLine,
            None,
            "invalid method token",
        ))
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
            Method::Info => "INFO",
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

    /// Returns the `CSeq` value as `(number, method)`.
    ///
    /// Returns an error if the header is missing, has an invalid sequence
    /// number, is missing the method token, or has an unknown method token.
    pub fn cseq(&self) -> Result<(u32, Method), SipError> {
        let value = self
            .headers()
            .get(&HeaderName::CSeq)
            .ok_or_else(|| {
                SipError::new(
                    SipErrorKind::MissingRequiredHeader,
                    None,
                    "missing CSeq header",
                )
            })?
            .as_str();
        let mut parts = value.splitn(2, char::is_whitespace);
        let num =
            parts.next().unwrap_or(value).parse::<u32>().map_err(|_| {
                SipError::new(SipErrorKind::InvalidHeader, None, "invalid CSeq number")
            })?;
        let method_str = parts.next().ok_or_else(|| {
            SipError::new(SipErrorKind::InvalidHeader, None, "missing CSeq method")
        })?;
        let method = Method::parse(method_str)
            .map_err(|_| SipError::new(SipErrorKind::InvalidHeader, None, "invalid CSeq method"))?;
        Ok((num, method))
    }

    /// Returns `Content-Length` header value.
    ///
    /// Returns an error if the header is missing or has a non-numeric value.
    pub fn content_length(&self) -> Result<usize, SipError> {
        let value = self
            .headers()
            .get(&HeaderName::ContentLength)
            .ok_or_else(|| {
                SipError::new(
                    SipErrorKind::MissingRequiredHeader,
                    None,
                    "missing Content-Length header",
                )
            })?
            .as_str()
            .trim();
        value
            .parse::<usize>()
            .map_err(|_| SipError::new(SipErrorKind::InvalidHeader, None, "invalid Content-Length"))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::sip::headers::HeaderValue;
    use crate::sip::uri::SipUri;

    fn message_with_cseq(value: &str) -> SipMessage {
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
        headers.append(HeaderName::CSeq, HeaderValue::new(value));
        SipMessage::Response {
            line: StatusLine::new(200, "OK"),
            headers,
            body: Vec::new(),
        }
    }

    fn message_with_content_length(value: &str) -> SipMessage {
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
        headers.append(HeaderName::ContentLength, HeaderValue::new(value));
        SipMessage::Request {
            line: RequestLine::new(
                Method::Invite,
                SipUri::parse("sip:user@example.com").unwrap(),
            ),
            headers,
            body: Vec::new(),
        }
    }

    #[test]
    fn cseq_parses_valid_header() {
        let msg = message_with_cseq("1 INVITE");
        assert_eq!(msg.cseq().unwrap(), (1, Method::Invite));
    }

    #[test]
    fn cseq_rejects_missing_header() {
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
        let msg = SipMessage::Response {
            line: StatusLine::new(200, "OK"),
            headers,
            body: Vec::new(),
        };
        assert_eq!(
            msg.cseq().unwrap_err().kind,
            SipErrorKind::MissingRequiredHeader
        );
    }

    #[test]
    fn cseq_rejects_invalid_number() {
        let msg = message_with_cseq("abc INVITE");
        assert_eq!(msg.cseq().unwrap_err().kind, SipErrorKind::InvalidHeader);
    }

    #[test]
    fn cseq_rejects_missing_method() {
        let msg = message_with_cseq("1");
        assert_eq!(msg.cseq().unwrap_err().kind, SipErrorKind::InvalidHeader);
    }

    #[test]
    fn cseq_rejects_invalid_method_token() {
        let msg = message_with_cseq("1 foo bar");
        assert_eq!(msg.cseq().unwrap_err().kind, SipErrorKind::InvalidHeader);
    }

    #[test]
    fn content_length_parses_valid_header() {
        let msg = message_with_content_length("42");
        assert_eq!(msg.content_length().unwrap(), 42);
    }

    #[test]
    fn content_length_rejects_missing_header() {
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
        let msg = SipMessage::Request {
            line: RequestLine::new(
                Method::Invite,
                SipUri::parse("sip:user@example.com").unwrap(),
            ),
            headers,
            body: Vec::new(),
        };
        assert_eq!(
            msg.content_length().unwrap_err().kind,
            SipErrorKind::MissingRequiredHeader
        );
    }

    #[test]
    fn content_length_rejects_invalid_number() {
        let msg = message_with_content_length("abc");
        assert_eq!(
            msg.content_length().unwrap_err().kind,
            SipErrorKind::InvalidHeader
        );
    }

    #[test]
    fn parse_normalized_accepts_lower_case_known_methods() {
        assert_eq!(
            Method::parse_normalized("register").unwrap(),
            Method::Register
        );
        assert_eq!(Method::parse_normalized("invite").unwrap(), Method::Invite);
        assert_eq!(Method::parse_normalized("BYE").unwrap(), Method::Bye);
    }

    #[test]
    fn parse_normalized_upper_cases_unknown_token() {
        assert_eq!(
            Method::parse_normalized("foo-bar").unwrap(),
            Method::Other("FOO-BAR".to_string())
        );
    }

    #[test]
    fn parse_normalized_rejects_empty_and_invalid_tokens() {
        assert!(Method::parse_normalized("").is_err());
        assert!(Method::parse_normalized("foo bar").is_err());
    }
}

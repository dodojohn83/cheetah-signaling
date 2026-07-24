//! SIP header names and ordered multi-value storage.

use super::error::{SipError, SipErrorKind};
use super::message::Method;
use super::uri::SipUri;
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Maximum byte length of a SIP header name stored in `HeaderName::Other`.
const MAX_HEADER_NAME_BYTES: usize = 128;

/// Truncates `s` at a UTF-8 character boundary so it is at most `max` bytes.
fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// Bounds `name` to [`MAX_HEADER_NAME_BYTES`] before case-insensitive matching.
fn bounded_header_name(name: Cow<'_, str>) -> Cow<'_, str> {
    if name.len() > MAX_HEADER_NAME_BYTES {
        let n = truncate_at_char_boundary(name.as_ref(), MAX_HEADER_NAME_BYTES);
        Cow::Owned(n.to_string())
    } else {
        name
    }
}

/// A SIP header value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeaderValue(String);

impl HeaderValue {
    /// Creates a header value from text.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Raw value bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Value as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HeaderValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl HeaderValue {
    /// Builds a `Via` header value of the form
    /// `SIP/2.0/{transport} {host}:{port};branch={branch}`.
    pub fn via(transport: &str, host: &str, port: u16, branch: &str) -> Result<Self, SipError> {
        if host.is_empty() {
            return Err(SipError::new(
                SipErrorKind::InvalidHeader,
                None,
                "Via host must not be empty",
            ));
        }
        validate_token(transport, "Via transport")?;
        validate_token(branch, "Via branch")?;
        Ok(Self(format!(
            "SIP/2.0/{transport} {host}:{port};branch={branch}"
        )))
    }

    /// Builds a `From` header value of the form `<{uri}>;tag={tag}`.
    pub fn from_uri(uri: &SipUri, tag: &str) -> Result<Self, SipError> {
        validate_token(tag, "From tag")?;
        Ok(Self(format!("<{}>;tag={tag}", uri.encode())))
    }

    /// Builds a `To` header value of the form `<{uri}>`.
    pub fn to_uri(uri: &SipUri) -> Self {
        Self(format!("<{}>", uri.encode()))
    }

    /// Builds a `Contact` header value of the form `<{uri}>`.
    pub fn contact_uri(uri: &SipUri) -> Self {
        Self(format!("<{}>", uri.encode()))
    }

    /// Builds a `CSeq` header value of the form `{seq} {method}`.
    pub fn cseq(seq: u32, method: Method) -> Self {
        Self(format!("{seq} {method}"))
    }
}

fn validate_token(value: &str, what: &str) -> Result<(), SipError> {
    if value.is_empty() || !value.bytes().all(is_token_char) {
        return Err(SipError::new(
            SipErrorKind::InvalidHeader,
            None,
            format!("{what} is not a valid SIP token"),
        ));
    }
    Ok(())
}

fn is_token_char(b: u8) -> bool {
    matches!(
        b,
        b'a'..=b'z'
        | b'A'..=b'Z'
        | b'0'..=b'9'
        | b'-' | b'.' | b'!' | b'%' | b'*' | b'_'
        | b'+' | b'`' | b'\'' | b'~' | b'(' | b')'
    )
}

/// Well-known SIP header names.
///
/// Unknown headers are stored as `Other` preserving original casing. Equality,
/// ordering, and hashing are case-insensitive for `Other` variants so that
/// `BTreeMap` lookups respect RFC 3261's case-insensitive header-name rule.
#[derive(Clone, Debug)]
pub enum HeaderName {
    /// Via.
    Via,
    /// From.
    From,
    /// To.
    To,
    /// Call-ID.
    CallId,
    /// CSeq.
    CSeq,
    /// Contact.
    Contact,
    /// Max-Forwards.
    MaxForwards,
    /// User-Agent.
    UserAgent,
    /// Content-Type.
    ContentType,
    /// Content-Length.
    ContentLength,
    /// Expires.
    Expires,
    /// Route.
    Route,
    /// Record-Route.
    RecordRoute,
    /// Authorization.
    Authorization,
    /// WWW-Authenticate.
    WwwAuthenticate,
    /// Proxy-Authenticate.
    ProxyAuthenticate,
    /// Proxy-Authorization.
    ProxyAuthorization,
    /// Subject (GB28181 catalog subscribe/notify).
    Subject,
    /// An unrecognized header.
    Other(String),
}

impl HeaderName {
    /// Parses a header name case-insensitively.
    pub fn parse(name: &str) -> Self {
        Self::from_cow(Cow::Borrowed(name))
    }

    fn from_cow(name: Cow<'_, str>) -> Self {
        let name = bounded_header_name(name);
        if name.eq_ignore_ascii_case("via") || name.eq_ignore_ascii_case("v") {
            HeaderName::Via
        } else if name.eq_ignore_ascii_case("from") || name.eq_ignore_ascii_case("f") {
            HeaderName::From
        } else if name.eq_ignore_ascii_case("to") || name.eq_ignore_ascii_case("t") {
            HeaderName::To
        } else if name.eq_ignore_ascii_case("call-id") || name.eq_ignore_ascii_case("i") {
            HeaderName::CallId
        } else if name.eq_ignore_ascii_case("cseq") {
            HeaderName::CSeq
        } else if name.eq_ignore_ascii_case("contact") || name.eq_ignore_ascii_case("m") {
            HeaderName::Contact
        } else if name.eq_ignore_ascii_case("max-forwards") {
            HeaderName::MaxForwards
        } else if name.eq_ignore_ascii_case("user-agent") {
            HeaderName::UserAgent
        } else if name.eq_ignore_ascii_case("content-type") || name.eq_ignore_ascii_case("c") {
            HeaderName::ContentType
        } else if name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("l") {
            HeaderName::ContentLength
        } else if name.eq_ignore_ascii_case("expires") {
            HeaderName::Expires
        } else if name.eq_ignore_ascii_case("route") {
            HeaderName::Route
        } else if name.eq_ignore_ascii_case("record-route") {
            HeaderName::RecordRoute
        } else if name.eq_ignore_ascii_case("authorization") {
            HeaderName::Authorization
        } else if name.eq_ignore_ascii_case("www-authenticate") {
            HeaderName::WwwAuthenticate
        } else if name.eq_ignore_ascii_case("proxy-authenticate") {
            HeaderName::ProxyAuthenticate
        } else if name.eq_ignore_ascii_case("proxy-authorization") {
            HeaderName::ProxyAuthorization
        } else if name.eq_ignore_ascii_case("subject") || name.eq_ignore_ascii_case("s") {
            HeaderName::Subject
        } else {
            HeaderName::Other(name.into_owned())
        }
    }

    /// Returns the canonical wire form of the header name.
    pub fn as_str(&self) -> &str {
        match self {
            HeaderName::Via => "Via",
            HeaderName::From => "From",
            HeaderName::To => "To",
            HeaderName::CallId => "Call-ID",
            HeaderName::CSeq => "CSeq",
            HeaderName::Contact => "Contact",
            HeaderName::MaxForwards => "Max-Forwards",
            HeaderName::UserAgent => "User-Agent",
            HeaderName::ContentType => "Content-Type",
            HeaderName::ContentLength => "Content-Length",
            HeaderName::Expires => "Expires",
            HeaderName::Route => "Route",
            HeaderName::RecordRoute => "Record-Route",
            HeaderName::Authorization => "Authorization",
            HeaderName::WwwAuthenticate => "WWW-Authenticate",
            HeaderName::ProxyAuthenticate => "Proxy-Authenticate",
            HeaderName::ProxyAuthorization => "Proxy-Authorization",
            HeaderName::Subject => "Subject",
            HeaderName::Other(s) => s.as_str(),
        }
    }
}

impl HeaderName {
    fn discriminant(&self) -> u8 {
        match self {
            HeaderName::Via => 0,
            HeaderName::From => 1,
            HeaderName::To => 2,
            HeaderName::CallId => 3,
            HeaderName::CSeq => 4,
            HeaderName::Contact => 5,
            HeaderName::MaxForwards => 6,
            HeaderName::UserAgent => 7,
            HeaderName::ContentType => 8,
            HeaderName::ContentLength => 9,
            HeaderName::Expires => 10,
            HeaderName::Route => 11,
            HeaderName::RecordRoute => 12,
            HeaderName::Authorization => 13,
            HeaderName::WwwAuthenticate => 14,
            HeaderName::ProxyAuthenticate => 15,
            HeaderName::ProxyAuthorization => 16,
            HeaderName::Subject => 17,
            HeaderName::Other(_) => 18,
        }
    }
}

fn case_insensitive_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let a = a.as_bytes().iter().map(|b| b.to_ascii_lowercase());
    let b = b.as_bytes().iter().map(|b| b.to_ascii_lowercase());
    a.cmp(b)
}

impl PartialEq for HeaderName {
    fn eq(&self, other: &Self) -> bool {
        self.discriminant() == other.discriminant()
            && match (self, other) {
                (HeaderName::Other(a), HeaderName::Other(b)) => {
                    case_insensitive_cmp(a, b) == std::cmp::Ordering::Equal
                }
                _ => true,
            }
    }
}

impl Eq for HeaderName {}

impl std::hash::Hash for HeaderName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.discriminant().hash(state);
        if let HeaderName::Other(s) = self {
            for b in s.as_bytes() {
                b.to_ascii_lowercase().hash(state);
            }
        }
    }
}

impl PartialOrd for HeaderName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeaderName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.discriminant().cmp(&other.discriminant()) {
            std::cmp::Ordering::Equal => match (self, other) {
                (HeaderName::Other(a), HeaderName::Other(b)) => case_insensitive_cmp(a, b),
                _ => std::cmp::Ordering::Equal,
            },
            other => other,
        }
    }
}

impl std::fmt::Display for HeaderName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl PartialEq<str> for HeaderName {
    fn eq(&self, other: &str) -> bool {
        self.as_str().eq_ignore_ascii_case(other)
    }
}

/// Ordered collection of SIP headers preserving insertion order and duplicates.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SipHeaders {
    headers: Vec<(HeaderName, HeaderValue)>,
    index: BTreeMap<HeaderName, Vec<usize>>,
}

impl SipHeaders {
    /// Creates empty headers.
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            index: BTreeMap::new(),
        }
    }

    /// Appends a header preserving order and allowing duplicates.
    pub fn append(&mut self, name: HeaderName, value: HeaderValue) {
        let idx = self.headers.len();
        self.index.entry(name.clone()).or_default().push(idx);
        self.headers.push((name, value));
    }

    /// Returns the first value for a header, if any.
    pub fn get(&self, name: &HeaderName) -> Option<&HeaderValue> {
        self.index
            .get(name)
            .and_then(|v| v.first())
            .and_then(|&idx| self.headers.get(idx))
            .map(|(_, value)| value)
    }

    /// Returns all values for a header in order.
    pub fn get_all(&self, name: &HeaderName) -> impl Iterator<Item = &HeaderValue> + '_ {
        self.index
            .get(name)
            .into_iter()
            .flat_map(|indices| indices.iter())
            .filter_map(move |&idx| self.headers.get(idx).map(|(_, v)| v))
    }

    /// Iterates over all headers in order.
    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> + '_ {
        self.headers.iter().map(|(n, v)| (n, v))
    }

    /// Returns the number of header lines.
    pub fn len(&self) -> usize {
        self.headers.len()
    }

    /// Returns true if there are no headers.
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }

    /// Encodes headers to `\r\n`-terminated lines.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        for (name, value) in &self.headers {
            out.push_str(name.as_str());
            out.push_str(": ");
            out.push_str(value.as_str());
            out.push_str("\r\n");
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn via_rejects_invalid_branch_token() {
        let result = HeaderValue::via("UDP", "example.com", 5060, "branch;bad");
        assert!(result.is_err());
    }

    #[test]
    fn from_uri_rejects_invalid_tag() {
        let uri = SipUri::parse("sip:alice@example.com:5060").unwrap();
        let result = HeaderValue::from_uri(&uri, "tag bad");
        assert!(result.is_err());
    }

    #[test]
    fn compact_header_forms_map_to_canonical_names() {
        let cases = [
            ("v", HeaderName::Via),
            ("f", HeaderName::From),
            ("t", HeaderName::To),
            ("i", HeaderName::CallId),
            ("m", HeaderName::Contact),
            ("c", HeaderName::ContentType),
            ("l", HeaderName::ContentLength),
            ("s", HeaderName::Subject),
        ];
        for (compact, expected) in cases {
            assert_eq!(HeaderName::parse(compact), expected);
            // Compact forms are case-insensitive like all header names.
            assert_eq!(HeaderName::parse(&compact.to_ascii_uppercase()), expected);
        }
    }

    #[test]
    fn unknown_header_is_preserved_case_insensitively() {
        let a = HeaderName::parse("X-Vendor-Tag");
        let b = HeaderName::parse("x-vendor-tag");
        assert!(matches!(a, HeaderName::Other(ref s) if s == "X-Vendor-Tag"));
        // Original casing is preserved for serialization ...
        assert_eq!(a.as_str(), "X-Vendor-Tag");
        // ... but equality is case-insensitive per RFC 3261.
        assert_eq!(a, b);
    }

    #[test]
    fn header_name_is_truncated_to_max_bytes() {
        let long = "x".repeat(MAX_HEADER_NAME_BYTES + 100);
        let h = HeaderName::parse(&long);
        assert!(matches!(h, HeaderName::Other(ref s) if s.len() <= MAX_HEADER_NAME_BYTES));
    }

    #[test]
    fn header_name_multibyte_truncation_respects_char_boundaries() {
        // 50 three-byte characters = 150 bytes. Truncating to 128 must land
        // on a valid UTF-8 boundary, not panic or produce invalid UTF-8.
        let long = "中".repeat(50);
        let _ = HeaderName::parse(&long); // must not panic
    }

    #[test]
    fn structured_headers_encode_correctly() {
        let uri = SipUri::parse("sip:alice@example.com:5060").unwrap();
        let via = HeaderValue::via("UDP", "example.com", 5060, "z9hG4bKabc").unwrap();
        let from = HeaderValue::from_uri(&uri, "abc123").unwrap();
        let to = HeaderValue::to_uri(&uri);
        let contact = HeaderValue::contact_uri(&uri);
        let cseq = HeaderValue::cseq(42, Method::Register);

        assert_eq!(
            via.as_str(),
            "SIP/2.0/UDP example.com:5060;branch=z9hG4bKabc"
        );
        assert_eq!(from.as_str(), "<sip:alice@example.com:5060>;tag=abc123");
        assert_eq!(to.as_str(), "<sip:alice@example.com:5060>");
        assert_eq!(contact.as_str(), "<sip:alice@example.com:5060>");
        assert_eq!(cseq.as_str(), "42 REGISTER");
    }
}

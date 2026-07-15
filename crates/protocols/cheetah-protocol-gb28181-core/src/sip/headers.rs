//! SIP header names and ordered multi-value storage.

use std::borrow::Cow;
use std::collections::BTreeMap;

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

/// Well-known SIP header names.
///
/// Unknown headers are stored as `Other` preserving original casing.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
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
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "via" => HeaderName::Via,
            "from" => HeaderName::From,
            "to" => HeaderName::To,
            "call-id" | "i" => HeaderName::CallId,
            "cseq" => HeaderName::CSeq,
            "contact" | "m" => HeaderName::Contact,
            "max-forwards" => HeaderName::MaxForwards,
            "user-agent" => HeaderName::UserAgent,
            "content-type" | "c" => HeaderName::ContentType,
            "content-length" | "l" => HeaderName::ContentLength,
            "expires" => HeaderName::Expires,
            "route" => HeaderName::Route,
            "record-route" => HeaderName::RecordRoute,
            "authorization" => HeaderName::Authorization,
            "www-authenticate" => HeaderName::WwwAuthenticate,
            "proxy-authenticate" => HeaderName::ProxyAuthenticate,
            "proxy-authorization" => HeaderName::ProxyAuthorization,
            "subject" => HeaderName::Subject,
            _ => HeaderName::Other(name.into_owned()),
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

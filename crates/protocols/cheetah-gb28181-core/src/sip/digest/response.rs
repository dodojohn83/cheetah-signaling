//! Parsed `Authorization` digest response and server-generated challenge types.

use crate::sip::error::{SipError, SipErrorKind};
use sha2::Digest;
use std::borrow::Cow;
use std::fmt;

/// Digest algorithm selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestAlgorithm {
    /// RFC 2617 legacy; allowed only when explicitly enabled.
    Md5,
    /// RFC 7616 SHA-256.
    Sha256,
    /// RFC 7616 SHA-512.
    Sha512,
}

impl DigestAlgorithm {
    /// Parses the algorithm name from the wire form.
    pub fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("md5") {
            Some(Self::Md5)
        } else if s.eq_ignore_ascii_case("sha-256") || s.eq_ignore_ascii_case("sha256") {
            Some(Self::Sha256)
        } else if s.eq_ignore_ascii_case("sha-512") || s.eq_ignore_ascii_case("sha512") {
            Some(Self::Sha512)
        } else {
            None
        }
    }

    pub(crate) fn as_wire(&self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha256 => "SHA-256",
            Self::Sha512 => "SHA-512",
        }
    }

    pub(crate) fn hash_hex(&self, data: &[u8]) -> String {
        match self {
            Self::Md5 => hex::encode(*md5::compute(data)),
            Self::Sha256 => hex::encode(sha2::Sha256::digest(data)),
            Self::Sha512 => hex::encode(sha2::Sha512::digest(data)),
        }
    }
}

/// Quality-of-protection mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestQop {
    /// `auth`; protects only the request line and headers.
    Auth,
    /// `auth-int`; also protects the entity body. Not supported for validation.
    AuthInt,
}

impl DigestQop {
    fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("auth") {
            Some(Self::Auth)
        } else if s.eq_ignore_ascii_case("auth-int") {
            Some(Self::AuthInt)
        } else {
            None
        }
    }

    pub(crate) fn as_wire(&self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::AuthInt => "auth-int",
        }
    }
}

/// Error returned by digest operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DigestError {
    /// Header value could not be parsed.
    Malformed(String),
    /// Algorithm is not implemented (e.g. `MD5-sess` or an unknown token).
    UnknownAlgorithm,
    /// MD5 was used but is not allowed by policy.
    AlgorithmNotAllowed,
    /// The response algorithm does not match the algorithm the server offered
    /// in its challenge. Treated as a downgrade attempt and always rejected.
    AlgorithmDowngrade,
    /// `qop` value is unsupported or inconsistent with `nc`/`cnonce`.
    InvalidQop,
    /// Nonce signature could not be verified.
    InvalidNonce,
    /// Server secret is too short to generate secure nonce signatures.
    WeakSecret,
    /// Nonce signature is valid but the nonce has expired.
    StaleNonce,
    /// Replay of a previously seen `nc` value.
    ReplayDetected,
    /// Realm in the response does not match the configured realm.
    RealmMismatch,
    /// URI in the response does not match the request URI.
    UriMismatch,
    /// Computed response does not match.
    InvalidResponse,
}

impl fmt::Display for DigestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(s) => write!(f, "malformed digest header: {s}"),
            Self::UnknownAlgorithm => f.write_str("unknown digest algorithm"),
            Self::AlgorithmNotAllowed => f.write_str("MD5 not allowed by policy"),
            Self::AlgorithmDowngrade => f.write_str("digest algorithm downgrade rejected"),
            Self::InvalidQop => f.write_str("invalid or unsupported qop"),
            Self::InvalidNonce => f.write_str("nonce signature verification failed"),
            Self::WeakSecret => f.write_str("server secret is too short"),
            Self::StaleNonce => f.write_str("nonce is stale"),
            Self::ReplayDetected => f.write_str("digest replay detected"),
            Self::RealmMismatch => f.write_str("digest realm mismatch"),
            Self::UriMismatch => f.write_str("digest uri mismatch"),
            Self::InvalidResponse => f.write_str("digest response mismatch"),
        }
    }
}

impl std::error::Error for DigestError {}

impl From<DigestError> for SipError {
    fn from(e: DigestError) -> Self {
        SipError::new(SipErrorKind::AuthenticationFailure, None, e.to_string())
    }
}

/// Parsed `Authorization` Digest response.
#[derive(Clone, Eq, PartialEq)]
pub struct DigestResponse {
    /// username quoted in the response.
    pub username: String,
    /// realm quoted in the response.
    pub realm: String,
    /// nonce from the challenge.
    pub nonce: String,
    /// URI used in the A2 computation.
    pub uri: String,
    /// Client-computed response digest.
    pub response: String,
    /// Client nonce, required when qop is present.
    pub cnonce: Option<String>,
    /// Nonce count, required when qop is present.
    pub nc: Option<u64>,
    /// QoP value from the response.
    pub qop: Option<DigestQop>,
    /// Algorithm used by the client.
    pub algorithm: Option<DigestAlgorithm>,
    /// Opaque value echoed from the challenge.
    pub opaque: Option<String>,
}

impl fmt::Debug for DigestResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestResponse")
            .field("username", &"[REDACTED]")
            .field("realm", &self.realm)
            .field("nonce", &"[REDACTED]")
            .field("uri", &"[REDACTED]")
            .field("response", &"[REDACTED]")
            .field("cnonce", &self.cnonce.as_ref().map(|_| "[REDACTED]"))
            .field("nc", &self.nc)
            .field("qop", &self.qop)
            .field("algorithm", &self.algorithm)
            .field("opaque", &"[REDACTED]")
            .finish()
    }
}

impl DigestResponse {
    /// Default maximum length for a `Digest` `Authorization` header value.
    pub const DEFAULT_MAX_HEADER_LEN: usize = 2048;

    /// Parses a `Digest ...` `Authorization` header value.
    pub fn parse(value: &str) -> Result<Self, DigestError> {
        Self::parse_with_limit(value, Self::DEFAULT_MAX_HEADER_LEN)
    }

    /// Parses a `Digest ...` `Authorization` header value, rejecting inputs
    /// longer than `max_len` bytes.
    pub fn parse_with_limit(value: &str, max_len: usize) -> Result<Self, DigestError> {
        if value.len() > max_len {
            return Err(DigestError::Malformed(format!(
                "digest header exceeds maximum length of {max_len}"
            )));
        }
        let value = value.trim();
        let value = if value.len() > 7
            && value.is_char_boundary(7)
            && value[..7].eq_ignore_ascii_case("digest ")
        {
            &value[7..]
        } else {
            value
        };

        let mut username = None;
        let mut realm = None;
        let mut nonce = None;
        let mut uri = None;
        let mut response = None;
        let mut cnonce = None;
        let mut nc = None;
        let mut qop = None;
        let mut algorithm = None;
        let mut opaque = None;

        for part in split_commas(value) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some(eq) = part.find('=') else {
                return Err(DigestError::Malformed(
                    "missing '=' in digest parameter".to_string(),
                ));
            };
            let key = part[..eq].trim();
            let raw = part[eq + 1..].trim();
            let value = unquote(raw);

            if key.eq_ignore_ascii_case("username") {
                username = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("realm") {
                realm = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("nonce") {
                nonce = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("uri") {
                uri = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("response") {
                response = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("cnonce") {
                cnonce = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("nc") {
                nc = Some(
                    u64::from_str_radix(value.as_ref(), 16)
                        .map_err(|_| DigestError::Malformed("bad nc value".to_string()))?,
                );
            } else if key.eq_ignore_ascii_case("qop") {
                qop = DigestQop::parse(value.as_ref())
                    .map(Some)
                    .ok_or(DigestError::InvalidQop)?;
            } else if key.eq_ignore_ascii_case("algorithm") {
                algorithm = Some(
                    DigestAlgorithm::parse(value.as_ref()).ok_or(DigestError::UnknownAlgorithm)?,
                );
            } else if key.eq_ignore_ascii_case("opaque") {
                opaque = Some(value.into_owned());
            }
        }

        Ok(Self {
            username: username
                .ok_or_else(|| DigestError::Malformed("missing username".to_string()))?,
            realm: realm.ok_or_else(|| DigestError::Malformed("missing realm".to_string()))?,
            nonce: nonce.ok_or_else(|| DigestError::Malformed("missing nonce".to_string()))?,
            uri: uri.ok_or_else(|| DigestError::Malformed("missing uri".to_string()))?,
            response: response
                .ok_or_else(|| DigestError::Malformed("missing response".to_string()))?,
            cnonce,
            nc,
            qop,
            algorithm,
            opaque,
        })
    }

    /// Encodes the response as the value of an `Authorization` header.
    pub fn to_header_value(&self) -> String {
        let mut out = String::from("Digest");
        let mut first = true;
        write_quoted_param(&mut out, &mut first, "username", &self.username);
        write_quoted_param(&mut out, &mut first, "realm", &self.realm);
        write_quoted_param(&mut out, &mut first, "nonce", &self.nonce);
        write_quoted_param(&mut out, &mut first, "uri", &self.uri);
        write_quoted_param(&mut out, &mut first, "response", &self.response);
        if let Some(cnonce) = &self.cnonce {
            write_quoted_param(&mut out, &mut first, "cnonce", cnonce);
        }
        if let Some(nc) = self.nc {
            write_raw_param(&mut out, &mut first, "nc", &format!("{nc:08x}"));
        }
        if let Some(qop) = self.qop {
            write_raw_param(&mut out, &mut first, "qop", qop.as_wire());
        }
        if let Some(algorithm) = self.algorithm {
            write_raw_param(&mut out, &mut first, "algorithm", algorithm.as_wire());
        }
        if let Some(opaque) = &self.opaque {
            write_quoted_param(&mut out, &mut first, "opaque", opaque);
        }
        out
    }
}

/// A server-generated `WWW-Authenticate` challenge.
#[derive(Clone, Eq, PartialEq)]
pub struct DigestChallenge {
    /// Authentication realm.
    pub realm: String,
    /// Signed nonce.
    pub nonce: String,
    /// Opaque value echoed by the client.
    pub opaque: Option<String>,
    /// Whether the previously supplied nonce was stale.
    pub stale: bool,
    /// Algorithm to use.
    pub algorithm: DigestAlgorithm,
    /// QoP offered.
    pub qop: Option<DigestQop>,
}

impl fmt::Debug for DigestChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestChallenge")
            .field("realm", &self.realm)
            .field("nonce", &"[REDACTED]")
            .field("opaque", &self.opaque.as_ref().map(|_| "[REDACTED]"))
            .field("stale", &self.stale)
            .field("algorithm", &self.algorithm)
            .field("qop", &self.qop)
            .finish()
    }
}

impl DigestChallenge {
    /// Default maximum length for a `WWW-Authenticate` header value.
    pub const DEFAULT_MAX_HEADER_LEN: usize = 2048;

    /// Parses a `WWW-Authenticate` Digest challenge.
    pub fn parse(value: &str) -> Result<Self, DigestError> {
        Self::parse_with_limit(value, Self::DEFAULT_MAX_HEADER_LEN)
    }

    /// Parses a `WWW-Authenticate` Digest challenge, rejecting inputs longer
    /// than `max_len` bytes.
    pub fn parse_with_limit(value: &str, max_len: usize) -> Result<Self, DigestError> {
        if value.len() > max_len {
            return Err(DigestError::Malformed(format!(
                "digest header exceeds maximum length of {max_len}"
            )));
        }
        let value = value.trim();
        let value = if value.len() > 7
            && value.is_char_boundary(7)
            && value[..7].eq_ignore_ascii_case("digest ")
        {
            &value[7..]
        } else {
            value
        };

        let mut realm = None;
        let mut nonce = None;
        let mut opaque = None;
        let mut stale = false;
        let mut algorithm = None;
        let mut qop = None;

        for part in split_commas(value) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some(eq) = part.find('=') else {
                return Err(DigestError::Malformed(
                    "missing '=' in digest parameter".to_string(),
                ));
            };
            let key = part[..eq].trim();
            let raw = part[eq + 1..].trim();
            let value = unquote(raw);

            if key.eq_ignore_ascii_case("realm") {
                realm = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("nonce") {
                nonce = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("opaque") {
                opaque = Some(value.into_owned());
            } else if key.eq_ignore_ascii_case("stale") {
                stale = value.eq_ignore_ascii_case("true");
            } else if key.eq_ignore_ascii_case("algorithm") {
                algorithm = Some(
                    DigestAlgorithm::parse(value.as_ref()).ok_or(DigestError::UnknownAlgorithm)?,
                );
            } else if key.eq_ignore_ascii_case("qop") {
                let mut selected = None;
                for token in split_commas(value.as_ref()) {
                    let token = token.trim();
                    if token.is_empty() {
                        continue;
                    }
                    let token = unquote(token);
                    if token.eq_ignore_ascii_case("auth") {
                        selected = Some(DigestQop::Auth);
                        break;
                    } else if token.eq_ignore_ascii_case("auth-int") {
                        selected = Some(DigestQop::AuthInt);
                    }
                }
                qop = selected;
            }
        }

        Ok(Self {
            realm: realm.ok_or_else(|| DigestError::Malformed("missing realm".to_string()))?,
            nonce: nonce.ok_or_else(|| DigestError::Malformed("missing nonce".to_string()))?,
            opaque,
            stale,
            algorithm: algorithm.unwrap_or(DigestAlgorithm::Md5),
            qop,
        })
    }

    /// Encodes the challenge as the value of a `WWW-Authenticate` header.
    pub fn to_header_value(&self) -> String {
        let mut out = String::from("Digest");
        let mut first = true;
        write_quoted_param(&mut out, &mut first, "realm", &self.realm);
        write_quoted_param(&mut out, &mut first, "nonce", &self.nonce);
        write_raw_param(&mut out, &mut first, "algorithm", self.algorithm.as_wire());
        if let Some(opaque) = &self.opaque {
            write_quoted_param(&mut out, &mut first, "opaque", opaque);
        }
        if self.stale {
            write_raw_param(&mut out, &mut first, "stale", "true");
        }
        if let Some(qop) = self.qop {
            write_quoted_param(&mut out, &mut first, "qop", qop.as_wire());
        }
        out
    }
}

fn write_quoted_param(out: &mut String, first: &mut bool, name: &str, value: &str) {
    write_separator(out, first);
    out.push_str(name);
    out.push_str("=\"");
    out.push_str(&sanitize_quoted_value(value));
    out.push('"');
}

fn write_raw_param(out: &mut String, first: &mut bool, name: &str, value: &str) {
    write_separator(out, first);
    out.push_str(name);
    out.push('=');
    out.push_str(value);
}

fn write_separator(out: &mut String, first: &mut bool) {
    if *first {
        out.push(' ');
        *first = false;
    } else {
        out.push_str(", ");
    }
}

fn sanitize_quoted_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], "")
}

fn split_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaping = false;

    for (i, c) in value.char_indices() {
        if escaping {
            escaping = false;
            continue;
        }
        if c == '\\' {
            escaping = true;
            continue;
        }
        if c == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if c == ',' && !in_quotes {
            parts.push(&value[start..i]);
            start = i + c.len_utf8();
        }
    }
    parts.push(&value[start..]);
    parts
}

fn unquote(value: &str) -> Cow<'_, str> {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        if inner.contains('\\') {
            Cow::Owned(unescape(inner))
        } else {
            Cow::Borrowed(inner)
        }
    } else {
        Cow::Borrowed(value)
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

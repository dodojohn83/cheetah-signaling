//! Parsed `Authorization` digest response and server-generated challenge types.

use crate::sip::error::{SipError, SipErrorKind};
use sha2::Digest;
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
        match s.to_ascii_lowercase().as_str() {
            "md5" => Some(Self::Md5),
            "sha-256" | "sha256" => Some(Self::Sha256),
            "sha-512" | "sha512" => Some(Self::Sha512),
            _ => None,
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
        match s.to_ascii_lowercase().as_str() {
            "auth" => Some(Self::Auth),
            "auth-int" => Some(Self::AuthInt),
            _ => None,
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
    /// Algorithm is not implemented.
    UnknownAlgorithm,
    /// MD5 was used but is not allowed by policy.
    AlgorithmNotAllowed,
    /// `qop` value is unsupported or inconsistent with `nc`/`cnonce`.
    InvalidQop,
    /// Nonce signature could not be verified.
    InvalidNonce,
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
            Self::InvalidQop => f.write_str("invalid or unsupported qop"),
            Self::InvalidNonce => f.write_str("nonce signature verification failed"),
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
        let value = if value.len() > 7 && value[..7].eq_ignore_ascii_case("digest ") {
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
            let key = part[..eq].trim().to_ascii_lowercase();
            let raw = part[eq + 1..].trim();
            let value = unquote(raw);

            match key.as_str() {
                "username" => username = Some(value.to_string()),
                "realm" => realm = Some(value.to_string()),
                "nonce" => nonce = Some(value.to_string()),
                "uri" => uri = Some(value.to_string()),
                "response" => response = Some(value.to_string()),
                "cnonce" => cnonce = Some(value.to_string()),
                "nc" => {
                    nc = Some(
                        u64::from_str_radix(value, 16)
                            .map_err(|_| DigestError::Malformed("bad nc value".to_string()))?,
                    );
                }
                "qop" => {
                    qop = DigestQop::parse(value)
                        .map(Some)
                        .ok_or(DigestError::InvalidQop)?;
                }
                "algorithm" => {
                    algorithm =
                        Some(DigestAlgorithm::parse(value).ok_or(DigestError::UnknownAlgorithm)?);
                }
                "opaque" => opaque = Some(value.to_string()),
                _ => {}
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
    /// Encodes the challenge as the value of a `WWW-Authenticate` header.
    pub fn to_header_value(&self) -> String {
        let mut out = format!(
            "Digest realm=\"{}\", nonce=\"{}\", algorithm={}",
            escape_quotes(&self.realm),
            escape_quotes(&self.nonce),
            self.algorithm.as_wire()
        );
        if let Some(opaque) = &self.opaque {
            out.push_str(&format!(", opaque=\"{}\"", escape_quotes(opaque)));
        }
        if self.stale {
            out.push_str(", stale=true");
        }
        if let Some(qop) = self.qop {
            out.push_str(&format!(", qop=\"{}\"", qop.as_wire()));
        }
        out
    }
}

fn split_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;

    for (i, c) in value.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == ',' && !in_quotes {
            parts.push(&value[start..i]);
            start = i + c.len_utf8();
        }
    }
    parts.push(&value[start..]);
    parts
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn escape_quotes(s: &str) -> String {
    s.replace('"', "\\\"")
}

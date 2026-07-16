//! Shared types and wire helpers for RFC 2617/7616 digest authentication.

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
    pub(crate) fn parse(s: &str) -> Option<Self> {
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

pub(super) fn split_commas(value: &str) -> Vec<&str> {
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

pub(super) fn unquote(value: &str) -> Cow<'_, str> {
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

pub(super) fn write_quoted_param(out: &mut String, first: &mut bool, name: &str, value: &str) {
    write_separator(out, first);
    out.push_str(name);
    out.push_str("=\"");
    out.push_str(&sanitize_quoted_value(value));
    out.push('"');
}

pub(super) fn write_raw_param(out: &mut String, first: &mut bool, name: &str, value: &str) {
    write_separator(out, first);
    out.push_str(name);
    out.push('=');
    out.push_str(value);
}

pub(super) fn write_separator(out: &mut String, first: &mut bool) {
    if *first {
        out.push(' ');
        *first = false;
    } else {
        out.push_str(", ");
    }
}

pub(super) fn sanitize_quoted_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], "")
}

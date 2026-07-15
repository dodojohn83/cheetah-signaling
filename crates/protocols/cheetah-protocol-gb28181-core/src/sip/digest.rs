//! RFC 2617/7616 Digest authentication for SIP.
//!
//! This is a Sans-I/O server-side implementation. It generates and validates
//! nonces, parses `Authorization` Digest responses, computes H(A1)/H(A2), and
//! performs constant-time response comparison. All time values are supplied by
//! the caller as a monotonically non-decreasing second counter.

use super::error::{SipError, SipErrorKind};
use crate::Method;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};
use std::collections::VecDeque;
use std::fmt;
use subtle::ConstantTimeEq;

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

    fn as_wire(&self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha256 => "SHA-256",
            Self::Sha512 => "SHA-512",
        }
    }

    fn hash_hex(&self, data: &[u8]) -> String {
        match self {
            Self::Md5 => hex::encode(*md5::compute(data)),
            Self::Sha256 => hex::encode(Sha256::digest(data)),
            Self::Sha512 => hex::encode(Sha512::digest(data)),
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

    fn as_wire(&self) -> &'static str {
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
    UnknownAlgorithm(String),
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
            Self::UnknownAlgorithm(s) => write!(f, "unknown digest algorithm: {s}"),
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
#[derive(Clone, Debug, Eq, PartialEq)]
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

impl DigestResponse {
    /// Parses a `Digest ...` `Authorization` header value.
    pub fn parse(value: &str) -> Result<Self, DigestError> {
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
                return Err(DigestError::Malformed(format!("missing '=' in '{part}'")));
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
                            .map_err(|_| DigestError::Malformed(format!("bad nc: {value}")))?,
                    );
                }
                "qop" => {
                    qop = DigestQop::parse(value)
                        .map(Some)
                        .ok_or_else(|| DigestError::Malformed(format!("unknown qop: {value}")))?;
                }
                "algorithm" => {
                    algorithm = DigestAlgorithm::parse(value)
                        .ok_or_else(|| DigestError::UnknownAlgorithm(value.to_string()))?
                        .into();
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
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// Server-side digest authentication context.
pub struct DigestContext {
    realm: String,
    secret: Vec<u8>,
    allow_md5: bool,
    preferred_algorithm: DigestAlgorithm,
    qop: Option<DigestQop>,
    nonce_ttl_seconds: u64,
    replay_cache_capacity: usize,
}

impl fmt::Debug for DigestContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestContext")
            .field("realm", &self.realm)
            .field("secret", &"[REDACTED]")
            .field("allow_md5", &self.allow_md5)
            .field("preferred_algorithm", &self.preferred_algorithm)
            .field("qop", &self.qop)
            .field("nonce_ttl_seconds", &self.nonce_ttl_seconds)
            .field("replay_cache_capacity", &self.replay_cache_capacity)
            .finish()
    }
}

impl DigestContext {
    /// Creates a new context with sensible defaults.
    pub fn new(realm: impl Into<String>, secret: impl Into<Vec<u8>>) -> Self {
        Self {
            realm: realm.into(),
            secret: secret.into(),
            allow_md5: true,
            preferred_algorithm: DigestAlgorithm::Md5,
            qop: Some(DigestQop::Auth),
            nonce_ttl_seconds: 300,
            replay_cache_capacity: 1024,
        }
    }

    /// Sets whether MD5 is allowed. Stronger algorithms use SHA-256/SHA-512.
    pub fn allow_md5(mut self, allow: bool) -> Self {
        self.allow_md5 = allow;
        self
    }

    /// Sets the preferred algorithm advertised in challenges.
    pub fn preferred_algorithm(mut self, alg: DigestAlgorithm) -> Self {
        self.preferred_algorithm = alg;
        self
    }

    /// Sets the offered QoP.
    pub fn qop(mut self, qop: Option<DigestQop>) -> Self {
        self.qop = qop;
        self
    }

    /// Sets nonce time-to-live in seconds.
    pub fn nonce_ttl_seconds(mut self, ttl: u64) -> Self {
        self.nonce_ttl_seconds = ttl;
        self
    }

    /// Sets the per-nonce replay cache capacity.
    pub fn replay_cache_capacity(mut self, cap: usize) -> Self {
        self.replay_cache_capacity = cap;
        self
    }

    /// Returns the configured realm.
    pub fn realm(&self) -> &str {
        &self.realm
    }

    /// Generates a new challenge for the given timestamp.
    pub fn generate_challenge(&self, now: u64) -> Result<DigestChallenge, DigestError> {
        Ok(DigestChallenge {
            realm: self.realm.clone(),
            nonce: generate_nonce(&self.secret, now)?,
            opaque: None,
            stale: false,
            algorithm: self.preferred_algorithm,
            qop: self.qop,
        })
    }

    /// Generates a stale challenge, typically used in a 401 after an expired
    /// nonce was supplied.
    pub fn generate_stale_challenge(&self, now: u64) -> Result<DigestChallenge, DigestError> {
        let mut c = self.generate_challenge(now)?;
        c.stale = true;
        Ok(c)
    }

    /// Validates a client response against a password and replay cache.
    pub fn validate(
        &self,
        response: &DigestResponse,
        method: &Method,
        uri: &str,
        password: &str,
        replay_cache: &mut DigestReplayCache,
        now: u64,
    ) -> Result<(), DigestError> {
        let algorithm = response.algorithm.unwrap_or(DigestAlgorithm::Md5);
        if algorithm == DigestAlgorithm::Md5 && !self.allow_md5 {
            return Err(DigestError::AlgorithmNotAllowed);
        }

        if response.realm != self.realm {
            return Err(DigestError::RealmMismatch);
        }
        if response.uri != uri {
            return Err(DigestError::UriMismatch);
        }

        if response.qop == Some(DigestQop::AuthInt) {
            return Err(DigestError::InvalidQop);
        }
        if response.qop == Some(DigestQop::Auth)
            && (response.nc.is_none() || response.cnonce.is_none())
        {
            return Err(DigestError::InvalidQop);
        }
        if response.qop.is_none() && (response.nc.is_some() || response.cnonce.is_some()) {
            return Err(DigestError::InvalidQop);
        }

        validate_nonce(&response.nonce, &self.secret, now, self.nonce_ttl_seconds)?;

        let nc = response.nc.unwrap_or(0);
        if !replay_cache.check(
            &response.nonce,
            nc,
            response.cnonce.as_deref(),
            now,
            self.nonce_ttl_seconds,
        ) {
            return Err(DigestError::ReplayDetected);
        }

        let method = method.to_string();
        let expected = compute_response(
            algorithm,
            &response.username,
            &response.realm,
            password,
            &response.nonce,
            nc,
            response.cnonce.as_deref(),
            response.qop,
            &method,
            uri,
        );

        let expected_bytes = hex::decode(&expected).map_err(|_| DigestError::InvalidResponse)?;
        let actual_bytes =
            hex::decode(&response.response).map_err(|_| DigestError::InvalidResponse)?;
        if actual_bytes.ct_eq(&expected_bytes).unwrap_u8() != 0 {
            Ok(())
        } else {
            Err(DigestError::InvalidResponse)
        }
    }
}

/// Bounded replay cache for digest `nc` values.
#[derive(Debug)]
pub struct DigestReplayCache {
    capacity: usize,
    entries: VecDeque<ReplayEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayEntry {
    nonce: String,
    nc: u64,
    cnonce: Option<String>,
    inserted_at: u64,
}

impl DigestReplayCache {
    /// Creates a cache with the given maximum number of stored entries.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity.min(1024)),
        }
    }

    /// Records the `nc` value and returns `true` if it is new, `false` if it
    /// has been seen before.
    pub fn check(
        &mut self,
        nonce: &str,
        nc: u64,
        cnonce: Option<&str>,
        now: u64,
        ttl: u64,
    ) -> bool {
        let cnonce = cnonce.map(|s| s.to_string());

        self.prune(now, ttl);

        for e in &self.entries {
            if e.nonce == nonce && e.nc == nc && e.cnonce == cnonce {
                return false;
            }
        }

        self.entries.push_back(ReplayEntry {
            nonce: nonce.to_string(),
            nc,
            cnonce,
            inserted_at: now,
        });
        if self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
        true
    }

    /// Removes entries older than `ttl` seconds relative to `now`.
    pub fn prune(&mut self, now: u64, ttl: u64) {
        while self
            .entries
            .front()
            .is_some_and(|e| e.inserted_at.saturating_add(ttl) < now)
        {
            self.entries.pop_front();
        }
    }
}

fn generate_nonce(secret: &[u8], timestamp: u64) -> Result<String, DigestError> {
    let ts_bytes = timestamp.to_be_bytes();
    let signature = nonce_signature(secret, &ts_bytes)?;
    let mut out = String::with_capacity(48);
    out.push_str(&hex::encode(ts_bytes));
    out.push_str(&hex::encode(&signature[..16]));
    Ok(out)
}

fn validate_nonce(nonce: &str, secret: &[u8], now: u64, ttl: u64) -> Result<u64, DigestError> {
    if nonce.len() != 48 || !nonce.is_ascii() {
        return Err(DigestError::InvalidNonce);
    }
    let bytes = hex::decode(nonce).map_err(|_| DigestError::InvalidNonce)?;
    if bytes.len() != 24 {
        return Err(DigestError::InvalidNonce);
    }
    let (ts_bytes, sig) = bytes.split_at(8);
    let ts_array: [u8; 8] = ts_bytes.try_into().map_err(|_| DigestError::InvalidNonce)?;
    let timestamp = u64::from_be_bytes(ts_array);

    let expected = nonce_signature(secret, ts_bytes)?;
    if sig.ct_eq(&expected[..16]).unwrap_u8() == 0 {
        return Err(DigestError::InvalidNonce);
    }

    if now.saturating_sub(timestamp) > ttl {
        return Err(DigestError::StaleNonce);
    }

    Ok(timestamp)
}

fn nonce_signature(secret: &[u8], timestamp: &[u8]) -> Result<Vec<u8>, DigestError> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| DigestError::InvalidNonce)?;
    mac.update(timestamp);
    Ok(mac.finalize().into_bytes().to_vec())
}

#[allow(clippy::too_many_arguments)]
fn compute_response(
    algorithm: DigestAlgorithm,
    username: &str,
    realm: &str,
    password: &str,
    nonce: &str,
    nc: u64,
    cnonce: Option<&str>,
    qop: Option<DigestQop>,
    method: &str,
    uri: &str,
) -> String {
    let a1 = format!("{username}:{realm}:{password}");
    let ha1 = algorithm.hash_hex(a1.as_bytes());

    let a2 = format!("{method}:{uri}");
    let ha2 = algorithm.hash_hex(a2.as_bytes());

    let a3 = match qop {
        Some(DigestQop::Auth) => {
            let cnonce = cnonce.unwrap_or("");
            let nc = format!("{nc:08x}");
            format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}")
        }
        _ => format!("{ha1}:{nonce}:{ha2}"),
    };
    algorithm.hash_hex(a3.as_bytes())
}

fn split_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut prev = 0;

    for (i, c) in value.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == ',' && !in_quotes {
            parts.push(&value[start..i]);
            start = i + c.len_utf8();
        }
        prev = i;
    }
    let _ = prev;
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

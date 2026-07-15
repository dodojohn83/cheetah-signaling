//! Server-side digest authentication context.

use super::nonce::{generate_nonce, validate_nonce};
use super::replay_cache::DigestReplayCache;
use super::response::{DigestAlgorithm, DigestChallenge, DigestError, DigestQop, DigestResponse};
use crate::Method;
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretBox, SecretString};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use subtle::ConstantTimeEq;

/// Server-side digest authentication context.
pub struct DigestContext {
    realm: String,
    secret: SecretBox<Vec<u8>>,
    nonce_counter: AtomicU64,
    allow_md5: bool,
    preferred_algorithm: DigestAlgorithm,
    qop: Option<DigestQop>,
    nonce_ttl_seconds: u64,
}

impl fmt::Debug for DigestContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestContext")
            .field("realm", &self.realm)
            .field("secret", &"[REDACTED]")
            .field("nonce_counter", &self.nonce_counter)
            .field("allow_md5", &self.allow_md5)
            .field("preferred_algorithm", &self.preferred_algorithm)
            .field("qop", &self.qop)
            .field("nonce_ttl_seconds", &self.nonce_ttl_seconds)
            .finish()
    }
}

impl DigestContext {
    /// Creates a new context with sensible defaults.
    ///
    /// MD5 is disabled by default because it is cryptographically broken. Call
    /// [`Self::allow_md5`] with `true` when interworking with legacy GB28181
    /// devices that cannot use SHA-256/SHA-512. The preferred challenge
    /// algorithm defaults to SHA-256.
    ///
    /// Returns an error if the supplied server secret is shorter than 32 bytes;
    /// short secrets make HMAC-SHA256 nonce signatures trivially brute-forceable.
    ///
    /// Carriage-return and line-feed characters are removed from the realm so
    /// that the stored value matches the wire value produced by
    /// [`DigestChallenge::to_header_value`], preventing spurious realm mismatches.
    pub fn new(realm: impl Into<String>, secret: impl AsRef<[u8]>) -> Result<Self, DigestError> {
        const MIN_SECRET_LEN: usize = 32;
        let realm = strip_crlf(&realm.into());
        let secret = secret.as_ref().to_vec();
        if secret.len() < MIN_SECRET_LEN {
            return Err(DigestError::WeakSecret);
        }
        Ok(Self {
            realm,
            secret: SecretBox::new(Box::new(secret)),
            nonce_counter: AtomicU64::new(0),
            allow_md5: false,
            preferred_algorithm: DigestAlgorithm::Sha256,
            qop: Some(DigestQop::Auth),
            nonce_ttl_seconds: 300,
        })
    }

    /// Sets whether MD5 is allowed. Stronger algorithms use SHA-256/SHA-512.
    ///
    /// Enable this only when interworking with legacy devices that do not
    /// support SHA-256/SHA-512.
    pub fn allow_md5(mut self, allow: bool) -> Self {
        self.allow_md5 = allow;
        self
    }

    /// Sets the preferred algorithm advertised in challenges.
    pub fn preferred_algorithm(mut self, alg: DigestAlgorithm) -> Self {
        self.preferred_algorithm = alg;
        self
    }

    /// Sets the offered QoP. `auth-int` is not supported and will be rejected.
    pub fn qop(mut self, qop: Option<DigestQop>) -> Result<Self, DigestError> {
        if qop == Some(DigestQop::AuthInt) {
            return Err(DigestError::InvalidQop);
        }
        self.qop = qop;
        Ok(self)
    }

    /// Sets nonce time-to-live in seconds.
    pub fn nonce_ttl_seconds(mut self, ttl: u64) -> Self {
        self.nonce_ttl_seconds = ttl;
        self
    }

    /// Returns the configured realm.
    pub fn realm(&self) -> &str {
        &self.realm
    }

    /// Generates a new challenge for the given timestamp.
    pub fn generate_challenge(&self, now: u64) -> Result<DigestChallenge, DigestError> {
        let counter = self.nonce_counter.fetch_add(1, Ordering::SeqCst);
        Ok(DigestChallenge {
            realm: self.realm.clone(),
            nonce: generate_nonce(self.secret.expose_secret().as_slice(), now, counter)?,
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
        password: &SecretString,
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
        if response.qop != self.qop {
            return Err(DigestError::InvalidQop);
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

        validate_nonce(
            &response.nonce,
            self.secret.expose_secret().as_slice(),
            now,
            self.nonce_ttl_seconds,
        )?;

        let nc = response.nc.unwrap_or(0);

        let method = method.to_string();
        let expected = compute_response(
            algorithm,
            &response.username,
            &response.realm,
            password.expose_secret(),
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
        if actual_bytes.ct_eq(&expected_bytes).unwrap_u8() == 0 {
            return Err(DigestError::InvalidResponse);
        }

        if response.qop.is_some()
            && !replay_cache.check(&response.nonce, nc, now, self.nonce_ttl_seconds)
        {
            return Err(DigestError::ReplayDetected);
        }

        Ok(())
    }
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
    let a1 = Zeroizing::new(format!("{username}:{realm}:{password}"));
    let ha1 = Zeroizing::new(algorithm.hash_hex(a1.as_bytes()));

    let a2 = format!("{method}:{uri}");
    let ha2 = algorithm.hash_hex(a2.as_bytes());

    let a3: Zeroizing<String> = match qop {
        Some(DigestQop::Auth) => {
            let cnonce = cnonce.unwrap_or("");
            let nc = format!("{nc:08x}");
            Zeroizing::new(format!("{}:{nonce}:{nc}:{cnonce}:auth:{ha2}", ha1.as_str()))
        }
        _ => Zeroizing::new(format!("{}:{nonce}:{ha2}", ha1.as_str())),
    };
    algorithm.hash_hex(a3.as_bytes())
}

fn strip_crlf(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '\r' | '\n')).collect()
}

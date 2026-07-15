//! Per-domain GB28181 access configuration.

use crate::error::AccessError;
use crate::types::DomainId;
use cheetah_gb28181_core::DigestAlgorithm;
use secrecy::{ExposeSecret, SecretBox};
use std::fmt;

/// Character-set handling for GB28181 XML bodies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharsetPolicy {
    /// Strict UTF-8 only.
    Utf8,
    /// Allow GB2312/GBK declarations and transcode to UTF-8 before parsing.
    GbkCompatible,
}

/// Authentication policy for the domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthPolicy {
    /// Require a successful digest exchange for every request.
    Required,
    /// Send a challenge but accept requests that do not present credentials.
    ChallengeOptional,
}

/// Configuration for a single GB28181 realm/tenant.
///
/// Fields are private to prevent external code from mutating security-relevant
/// settings (digest secret, authentication policy, algorithm preferences)
/// without validation. Use the constructor and `with_*` builder methods.
pub struct Gb28181DomainConfig {
    domain_id: DomainId,
    realm: String,
    digest_secret: SecretBox<Vec<u8>>,
    allow_md5: bool,
    preferred_algorithm: DigestAlgorithm,
    default_expires_seconds: u32,
    max_expires_seconds: u32,
    charset_policy: CharsetPolicy,
    heartbeat_timeout_seconds: u64,
    catalog_page_limit: usize,
    auth_policy: AuthPolicy,
}

impl Gb28181DomainConfig {
    /// Creates a default config for tests and bootstrapping.
    ///
    /// Returns an error if `domain_id` is not a valid [`DomainId`] or if the
    /// digest secret is shorter than 32 bytes.
    pub fn new(
        domain_id: impl AsRef<str>,
        realm: impl AsRef<str>,
        digest_secret: Vec<u8>,
    ) -> Result<Self, AccessError> {
        const MIN_SECRET_LEN: usize = 32;
        // Wrap the secret immediately so it is zeroized on any early-return path.
        let digest_secret = SecretBox::new(Box::new(digest_secret));
        let domain_id = DomainId::new(domain_id).ok_or(AccessError::InvalidDomainId)?;
        if digest_secret.expose_secret().len() < MIN_SECRET_LEN {
            return Err(AccessError::Internal("digest secret too short".to_string()));
        }
        Ok(Self {
            domain_id,
            realm: realm.as_ref().to_string(),
            digest_secret,
            allow_md5: false,
            preferred_algorithm: DigestAlgorithm::Sha256,
            default_expires_seconds: 3600,
            max_expires_seconds: 86400,
            charset_policy: CharsetPolicy::Utf8,
            heartbeat_timeout_seconds: 90,
            catalog_page_limit: 128,
            auth_policy: AuthPolicy::Required,
        })
    }

    /// Logical domain identifier used to select this config from a REGISTER
    /// Request-URI or To header host.
    pub fn domain_id(&self) -> &DomainId {
        &self.domain_id
    }

    /// SIP realm advertised in digest challenges.
    pub fn realm(&self) -> &str {
        &self.realm
    }

    /// Character-set policy for XML bodies.
    pub fn charset_policy(&self) -> CharsetPolicy {
        self.charset_policy
    }

    /// Default registration expiry in seconds.
    pub fn default_expires_seconds(&self) -> u32 {
        self.default_expires_seconds
    }

    /// Maximum registration expiry in seconds.
    pub fn max_expires_seconds(&self) -> u32 {
        self.max_expires_seconds
    }

    /// Heartbeat timeout in seconds (missing keepalive window).
    pub fn heartbeat_timeout_seconds(&self) -> u64 {
        self.heartbeat_timeout_seconds
    }

    /// Maximum items per catalog page.
    pub fn catalog_page_limit(&self) -> usize {
        self.catalog_page_limit
    }

    /// Authentication policy.
    pub fn auth_policy(&self) -> AuthPolicy {
        self.auth_policy
    }

    /// Whether legacy MD5 digest is allowed.
    pub fn allow_md5(&self) -> bool {
        self.allow_md5
    }

    /// Preferred digest algorithm advertised in challenges.
    pub fn preferred_algorithm(&self) -> DigestAlgorithm {
        self.preferred_algorithm
    }

    /// Returns the digest secret without exposing the underlying bytes.
    ///
    /// Callers must use `secrecy::ExposeSecret` to access the bytes.
    pub(crate) fn digest_secret(&self) -> &SecretBox<Vec<u8>> {
        &self.digest_secret
    }

    /// Returns a new config with the supplied authentication policy.
    pub fn with_auth_policy(mut self, policy: AuthPolicy) -> Self {
        self.auth_policy = policy;
        self
    }

    /// Returns a new config with MD5 allowed or disallowed.
    pub fn with_allow_md5(mut self, allow: bool) -> Self {
        self.allow_md5 = allow;
        self
    }

    /// Returns a new config with the preferred digest algorithm.
    pub fn with_preferred_algorithm(mut self, algorithm: DigestAlgorithm) -> Self {
        self.preferred_algorithm = algorithm;
        self
    }

    /// Returns a new config with a different digest secret.
    ///
    /// Returns `Err` if `secret` is shorter than 32 bytes.
    pub fn with_digest_secret(mut self, secret: Vec<u8>) -> Result<Self, AccessError> {
        const MIN_SECRET_LEN: usize = 32;
        // Wrap the secret immediately so it is zeroized on any early-return path.
        let secret = SecretBox::new(Box::new(secret));
        if secret.expose_secret().len() < MIN_SECRET_LEN {
            return Err(AccessError::Internal("digest secret too short".to_string()));
        }
        self.digest_secret = secret;
        Ok(self)
    }
}

impl Clone for Gb28181DomainConfig {
    fn clone(&self) -> Self {
        Self {
            domain_id: self.domain_id.clone(),
            realm: self.realm.clone(),
            digest_secret: SecretBox::new(Box::new(self.digest_secret.expose_secret().clone())),
            allow_md5: self.allow_md5,
            preferred_algorithm: self.preferred_algorithm,
            default_expires_seconds: self.default_expires_seconds,
            max_expires_seconds: self.max_expires_seconds,
            charset_policy: self.charset_policy,
            heartbeat_timeout_seconds: self.heartbeat_timeout_seconds,
            catalog_page_limit: self.catalog_page_limit,
            auth_policy: self.auth_policy,
        }
    }
}

impl fmt::Debug for Gb28181DomainConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gb28181DomainConfig")
            .field("domain_id", &self.domain_id)
            .field("realm", &self.realm)
            .field("digest_secret", &"[REDACTED]")
            .field("allow_md5", &self.allow_md5)
            .field("preferred_algorithm", &self.preferred_algorithm)
            .field("default_expires_seconds", &self.default_expires_seconds)
            .field("max_expires_seconds", &self.max_expires_seconds)
            .field("charset_policy", &self.charset_policy)
            .field("heartbeat_timeout_seconds", &self.heartbeat_timeout_seconds)
            .field("catalog_page_limit", &self.catalog_page_limit)
            .field("auth_policy", &self.auth_policy)
            .finish()
    }
}

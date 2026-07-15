//! Per-domain GB28181 access configuration.

use crate::error::AccessError;
use crate::types::DomainId;
use cheetah_protocol_gb28181_core::DigestAlgorithm;
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
pub struct Gb28181DomainConfig {
    /// Logical domain identifier used to select this config from a REGISTER
    /// Request-URI or To header host.
    pub domain_id: DomainId,
    /// SIP realm advertised in digest challenges.
    pub realm: String,
    /// Secret used to sign nonces and, when paired with a matching password,
    /// verify digest responses.
    pub digest_secret: SecretBox<Vec<u8>>,
    /// Whether to allow legacy MD5 digest. Stronger algorithms are preferred.
    pub allow_md5: bool,
    /// Preferred digest algorithm advertised in challenges.
    pub preferred_algorithm: DigestAlgorithm,
    /// Default registration expiry in seconds.
    pub default_expires_seconds: u32,
    /// Maximum registration expiry in seconds.
    pub max_expires_seconds: u32,
    /// Character-set policy for XML bodies.
    pub charset_policy: CharsetPolicy,
    /// Heartbeat timeout in seconds (missing keepalive window).
    pub heartbeat_timeout_seconds: u64,
    /// Maximum items per catalog page.
    pub catalog_page_limit: usize,
    /// Authentication policy.
    pub auth_policy: AuthPolicy,
}

impl Gb28181DomainConfig {
    /// Creates a default config for tests and bootstrapping.
    ///
    /// Returns an error if `domain_id` is not a valid [`DomainId`].
    pub fn new(
        domain_id: impl AsRef<str>,
        realm: impl AsRef<str>,
        digest_secret: Vec<u8>,
    ) -> Result<Self, AccessError> {
        let domain_id = DomainId::new(domain_id).ok_or(AccessError::InvalidDomainId)?;
        Ok(Self {
            domain_id,
            realm: realm.as_ref().to_string(),
            digest_secret: SecretBox::new(Box::new(digest_secret)),
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

    /// Returns the digest secret bytes for constructing a `DigestContext`.
    pub fn digest_secret_bytes(&self) -> Vec<u8> {
        self.digest_secret.expose_secret().clone()
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

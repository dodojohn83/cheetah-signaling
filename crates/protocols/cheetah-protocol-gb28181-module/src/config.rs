//! Per-domain GB28181 access configuration.

use cheetah_protocol_gb28181_core::DigestAlgorithm;

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
#[derive(Clone, Debug)]
pub struct Gb28181DomainConfig {
    /// Logical domain identifier used to select this config from a REGISTER
    /// Request-URI or To header host.
    pub domain_id: String,
    /// SIP realm advertised in digest challenges.
    pub realm: String,
    /// Secret used to sign nonces and, when paired with a matching password,
    /// verify digest responses.
    pub digest_secret: Vec<u8>,
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
    /// Production code should load values from configuration instead.
    pub fn new(
        domain_id: impl Into<String>,
        realm: impl Into<String>,
        digest_secret: Vec<u8>,
    ) -> Self {
        Self {
            domain_id: domain_id.into(),
            realm: realm.into(),
            digest_secret,
            allow_md5: false,
            preferred_algorithm: DigestAlgorithm::Sha256,
            default_expires_seconds: 3600,
            max_expires_seconds: 86400,
            charset_policy: CharsetPolicy::Utf8,
            heartbeat_timeout_seconds: 90,
            catalog_page_limit: 128,
            auth_policy: AuthPolicy::Required,
        }
    }
}

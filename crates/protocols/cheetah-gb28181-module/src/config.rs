//! Per-domain GB28181 access configuration.

use crate::error::AccessError;
use crate::types::DomainId;
use cheetah_domain::CompatibilityProfile;
use cheetah_gb28181_core::DigestAlgorithm;
use secrecy::{ExposeSecret, SecretSlice};
use std::fmt;

/// Maximum byte length of the SIP realm advertised in digest challenges.
const MAX_REALM_BYTES: usize = 64;

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
    digest_secret: SecretSlice<u8>,
    allow_md5: bool,
    preferred_algorithm: DigestAlgorithm,
    default_expires_seconds: u32,
    max_expires_seconds: u32,
    charset_policy: CharsetPolicy,
    heartbeat_timeout_seconds: u64,
    catalog_page_limit: usize,
    max_registrations: usize,
    auth_policy: AuthPolicy,
    auth_max_failures_per_source: u32,
    auth_rate_window_seconds: u64,
    auth_rate_max_sources: usize,
    compatibility: CompatibilityProfile,
}

impl Gb28181DomainConfig {
    /// Creates a default config for tests and bootstrapping.
    ///
    /// `digest_secret` must be a zeroizing secret type such as [`SecretSlice`].
    /// Callers are responsible for zeroizing any intermediate buffers used to
    /// construct the secret.
    ///
    /// Returns an error if `domain_id` is not a valid [`DomainId`] or if the
    /// digest secret is shorter than 32 bytes.
    pub fn new(
        domain_id: impl AsRef<str>,
        realm: impl AsRef<str>,
        digest_secret: impl Into<SecretSlice<u8>>,
    ) -> Result<Self, AccessError> {
        const MIN_SECRET_LEN: usize = 32;
        const DEFAULT_MAX_REGISTRATIONS: usize = 100_000;
        const DEFAULT_MAX_AUTH_FAILURES: u32 = 10;
        const DEFAULT_AUTH_RATE_WINDOW_SECONDS: u64 = 60;
        const DEFAULT_AUTH_RATE_MAX_SOURCES: usize = 65_536;
        // Consumes the secret into a zeroizing SecretSlice immediately.
        let digest_secret = digest_secret.into();
        let domain_id = DomainId::new(domain_id).ok_or(AccessError::InvalidDomainId)?;
        let realm = realm.as_ref();
        if realm.len() > MAX_REALM_BYTES {
            return Err(AccessError::Internal(
                "realm exceeds maximum length".to_string(),
            ));
        }
        if digest_secret.expose_secret().len() < MIN_SECRET_LEN {
            return Err(AccessError::Internal("digest secret too short".to_string()));
        }
        Ok(Self {
            domain_id,
            realm: realm.to_string(),
            digest_secret,
            allow_md5: false,
            preferred_algorithm: DigestAlgorithm::Sha256,
            default_expires_seconds: 3600,
            max_expires_seconds: 86400,
            charset_policy: CharsetPolicy::Utf8,
            heartbeat_timeout_seconds: 90,
            catalog_page_limit: 128,
            max_registrations: DEFAULT_MAX_REGISTRATIONS,
            auth_policy: AuthPolicy::Required,
            auth_max_failures_per_source: DEFAULT_MAX_AUTH_FAILURES,
            auth_rate_window_seconds: DEFAULT_AUTH_RATE_WINDOW_SECONDS,
            auth_rate_max_sources: DEFAULT_AUTH_RATE_MAX_SOURCES,
            compatibility: CompatibilityProfile::default(),
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

    /// Maximum number of simultaneous device registrations per domain.
    pub fn max_registrations(&self) -> usize {
        self.max_registrations
    }

    /// Maximum failed authentication attempts tolerated per source IP within
    /// [`auth_rate_window_seconds`](Self::auth_rate_window_seconds) before the
    /// source is temporarily rate-limited. Zero disables brute-force limiting.
    pub fn auth_max_failures_per_source(&self) -> u32 {
        self.auth_max_failures_per_source
    }

    /// Sliding window, in seconds, over which authentication failures are
    /// counted for brute-force rate limiting.
    pub fn auth_rate_window_seconds(&self) -> u64 {
        self.auth_rate_window_seconds
    }

    /// Maximum number of distinct source IPs tracked by the brute-force rate
    /// limiter. Bounds the limiter's memory use.
    pub fn auth_rate_max_sources(&self) -> usize {
        self.auth_rate_max_sources
    }

    /// Compatibility profile applied to parsing/encoding and endpoint decisions
    /// for this domain.
    pub fn compatibility(&self) -> &CompatibilityProfile {
        &self.compatibility
    }

    /// Returns the digest secret without exposing the underlying bytes.
    ///
    /// Callers must use `secrecy::ExposeSecret` to access the bytes.
    pub(crate) fn digest_secret(&self) -> &SecretSlice<u8> {
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
    /// `secret` must be a zeroizing secret type such as [`SecretSlice`].
    /// Returns `Err` if `secret` is shorter than 32 bytes.
    pub fn with_digest_secret(
        mut self,
        secret: impl Into<SecretSlice<u8>>,
    ) -> Result<Self, AccessError> {
        const MIN_SECRET_LEN: usize = 32;
        let secret = secret.into();
        if secret.expose_secret().len() < MIN_SECRET_LEN {
            return Err(AccessError::Internal("digest secret too short".to_string()));
        }
        self.digest_secret = secret;
        Ok(self)
    }

    /// Returns a new config with the supplied registration table capacity.
    pub fn with_max_registrations(mut self, max_registrations: usize) -> Self {
        self.max_registrations = max_registrations;
        self
    }

    /// Returns a new config with the supplied per-source authentication rate
    /// limit parameters.
    ///
    /// Setting `max_failures` or `max_sources` to zero disables brute-force
    /// rate limiting.
    pub fn with_auth_rate_limit(
        mut self,
        max_failures: u32,
        window_seconds: u64,
        max_sources: usize,
    ) -> Self {
        self.auth_max_failures_per_source = max_failures;
        self.auth_rate_window_seconds = window_seconds;
        self.auth_rate_max_sources = max_sources;
        self
    }

    /// Returns a new config with the supplied compatibility profile.
    pub fn with_compatibility(mut self, profile: CompatibilityProfile) -> Self {
        self.compatibility = profile;
        self
    }
}

impl Clone for Gb28181DomainConfig {
    fn clone(&self) -> Self {
        Self {
            domain_id: self.domain_id.clone(),
            realm: self.realm.clone(),
            digest_secret: self.digest_secret.clone(),
            allow_md5: self.allow_md5,
            preferred_algorithm: self.preferred_algorithm,
            default_expires_seconds: self.default_expires_seconds,
            max_expires_seconds: self.max_expires_seconds,
            charset_policy: self.charset_policy,
            heartbeat_timeout_seconds: self.heartbeat_timeout_seconds,
            catalog_page_limit: self.catalog_page_limit,
            max_registrations: self.max_registrations,
            auth_policy: self.auth_policy,
            auth_max_failures_per_source: self.auth_max_failures_per_source,
            auth_rate_window_seconds: self.auth_rate_window_seconds,
            auth_rate_max_sources: self.auth_rate_max_sources,
            compatibility: self.compatibility.clone(),
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
            .field("max_registrations", &self.max_registrations)
            .field("auth_policy", &self.auth_policy)
            .field(
                "auth_max_failures_per_source",
                &self.auth_max_failures_per_source,
            )
            .field("auth_rate_window_seconds", &self.auth_rate_window_seconds)
            .field("auth_rate_max_sources", &self.auth_rate_max_sources)
            .field("compatibility", &self.compatibility)
            .finish()
    }
}

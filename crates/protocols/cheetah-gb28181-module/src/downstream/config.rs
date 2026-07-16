//! Per-domain configuration for lower-platform GB28181 access.

use crate::config::AuthPolicy;
use crate::downstream::DownstreamError;
use crate::types::DomainId;
use cheetah_gb28181_core::{DigestAlgorithm, SipUri};
use secrecy::{ExposeSecret, SecretSlice};

/// Configuration for the local domain when acting as an upper platform to
/// lower-platform cascades.
#[derive(Clone, Debug)]
pub struct DownstreamConfig {
    domain_id: DomainId,
    local_uri: SipUri,
    realm: String,
    digest_secret: SecretSlice<u8>,
    allow_md5: bool,
    preferred_algorithm: DigestAlgorithm,
    default_expires_seconds: u32,
    max_expires_seconds: u32,
    heartbeat_timeout_seconds: u64,
    max_links: usize,
    auth_policy: AuthPolicy,
    catalog_page_limit: usize,
}

impl DownstreamConfig {
    /// Creates a validated downstream-platform configuration.
    ///
    /// `digest_secret` must be at least 32 bytes.
    pub fn new(
        domain_id: impl AsRef<str>,
        local_uri: impl AsRef<str>,
        realm: impl AsRef<str>,
        digest_secret: impl Into<SecretSlice<u8>>,
    ) -> Result<Self, DownstreamError> {
        const MIN_SECRET_LEN: usize = 32;
        const DEFAULT_MAX_LINKS: usize = 1_000;

        let domain_id = DomainId::new(domain_id).ok_or(DownstreamError::Access(
            crate::error::AccessError::InvalidDomainId,
        ))?;
        let local_uri = SipUri::parse(local_uri.as_ref()).map_err(|e| {
            DownstreamError::Access(crate::error::AccessError::Internal(format!(
                "invalid local URI: {e}"
            )))
        })?;
        let digest_secret = digest_secret.into();
        if digest_secret.expose_secret().len() < MIN_SECRET_LEN {
            return Err(DownstreamError::Access(
                crate::error::AccessError::Internal("digest secret too short".to_string()),
            ));
        }

        Ok(Self {
            domain_id,
            local_uri,
            realm: realm.as_ref().to_string(),
            digest_secret,
            allow_md5: false,
            preferred_algorithm: DigestAlgorithm::Sha256,
            default_expires_seconds: 3600,
            max_expires_seconds: 86400,
            heartbeat_timeout_seconds: 90,
            max_links: DEFAULT_MAX_LINKS,
            auth_policy: AuthPolicy::Required,
            catalog_page_limit: 128,
        })
    }

    /// Logical domain identifier.
    pub fn domain_id(&self) -> &DomainId {
        &self.domain_id
    }

    /// Local SIP URI used as the `From` address for outbound requests.
    pub fn local_uri(&self) -> &SipUri {
        &self.local_uri
    }

    /// Realm advertised in digest challenges.
    pub fn realm(&self) -> &str {
        &self.realm
    }

    /// Server secret used to sign nonces.
    pub(crate) fn digest_secret(&self) -> &SecretSlice<u8> {
        &self.digest_secret
    }

    /// Whether legacy MD5 digest is allowed.
    pub fn allow_md5(&self) -> bool {
        self.allow_md5
    }

    /// Preferred digest algorithm advertised in challenges.
    pub fn preferred_algorithm(&self) -> DigestAlgorithm {
        self.preferred_algorithm
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

    /// Maximum number of lower-platform links allowed to register.
    pub fn max_links(&self) -> usize {
        self.max_links
    }

    /// Authentication policy for lower-platform registration.
    pub fn auth_policy(&self) -> AuthPolicy {
        self.auth_policy
    }

    /// Maximum items per catalog page.
    pub fn catalog_page_limit(&self) -> usize {
        self.catalog_page_limit
    }

    /// Returns a config with the supplied link capacity.
    pub fn with_max_links(mut self, max_links: usize) -> Self {
        self.max_links = max_links;
        self
    }

    /// Returns a config with the supplied authentication policy.
    pub fn with_auth_policy(mut self, policy: AuthPolicy) -> Self {
        self.auth_policy = policy;
        self
    }

    /// Returns a config that allows or disallows MD5 digest.
    pub fn with_allow_md5(mut self, allow: bool) -> Self {
        self.allow_md5 = allow;
        self
    }

    /// Returns a config with the supplied preferred digest algorithm.
    pub fn with_preferred_algorithm(mut self, alg: DigestAlgorithm) -> Self {
        self.preferred_algorithm = alg;
        self
    }
}

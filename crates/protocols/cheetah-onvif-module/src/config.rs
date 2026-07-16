//! ONVIF module configuration.

/// Authentication policy for an ONVIF device.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum AuthPolicy {
    /// No authentication.
    None,
    /// HTTP Digest authentication.
    Digest,
    /// WS-Security UsernameToken over SOAP.
    #[default]
    UsernameToken,
}

/// Media service preference.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum MediaPreference {
    /// Prefer Media2, fall back to Media1.
    #[default]
    Media2ThenMedia1,
    /// Use Media1 only.
    Media1Only,
    /// Use Media2 only.
    Media2Only,
}

/// PullPoint subscription settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PullPointConfig {
    /// Initial subscription timeout in seconds.
    pub initial_timeout_seconds: u64,
    /// Renew interval in seconds.
    pub renew_interval_seconds: u64,
    /// Maximum messages returned per pull.
    pub max_messages: u32,
    /// Maximum response body size in bytes.
    pub max_body_bytes: usize,
}

impl Default for PullPointConfig {
    fn default() -> Self {
        Self {
            initial_timeout_seconds: 60,
            renew_interval_seconds: 30,
            max_messages: 100,
            max_body_bytes: 1_048_576,
        }
    }
}

/// Snapshot policy for an ONVIF device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotConfig {
    /// Maximum response size in bytes.
    pub max_size_bytes: usize,
    /// Cache TTL in seconds; zero disables caching.
    pub cache_ttl_seconds: u64,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            max_size_bytes: 8_388_608,
            cache_ttl_seconds: 0,
        }
    }
}

/// ONVIF module configuration for a tenant or device.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OnvifConfig {
    /// Authentication policy.
    pub auth_policy: AuthPolicy,
    /// Media service preference.
    pub media_preference: MediaPreference,
    /// PullPoint subscription settings.
    pub pull_point: PullPointConfig,
    /// Snapshot settings.
    pub snapshot: SnapshotConfig,
    /// Whether to allow domain-name hosts in discovered XAddrs; the driver is
    /// responsible for DNS resolution and DNS-rebinding validation.
    pub allow_domain_names: bool,
}

impl OnvifConfig {
    /// Creates a default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the authentication policy.
    pub fn with_auth_policy(mut self, auth_policy: AuthPolicy) -> Self {
        self.auth_policy = auth_policy;
        self
    }

    /// Sets the media preference.
    pub fn with_media_preference(mut self, media_preference: MediaPreference) -> Self {
        self.media_preference = media_preference;
        self
    }
}

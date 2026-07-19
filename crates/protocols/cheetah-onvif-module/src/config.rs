//! ONVIF module configuration.

pub use cheetah_onvif_core::discovery::XAddrPolicy;

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

/// XML response parsing limits to bound memory and CPU usage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParserLimits {
    /// Maximum element nesting depth.
    pub max_depth: usize,
    /// Maximum number of XML nodes (Start/Empty events) to read.
    pub max_nodes: usize,
    /// Maximum accumulated text length in bytes for a single element.
    pub max_text_bytes: usize,
    /// Maximum total input size in bytes.
    pub max_input_bytes: usize,
}

impl Default for ParserLimits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_nodes: 65_536,
            max_text_bytes: 65_536,
            max_input_bytes: 16_777_216,
        }
    }
}

/// ONVIF module configuration for a tenant or device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnvifConfig {
    /// Authentication policy.
    pub auth_policy: AuthPolicy,
    /// Media service preference.
    pub media_preference: MediaPreference,
    /// PullPoint subscription settings.
    pub pull_point: PullPointConfig,
    /// Snapshot settings.
    pub snapshot: SnapshotConfig,
    /// SSRF policy for discovered and service XAddrs.
    pub xaddr_policy: XAddrPolicy,
    /// Maximum number of devices kept in the provisioning workflow map.
    pub max_provisioning_state_entries: usize,
    /// XML parser limits.
    pub parser: ParserLimits,
}

impl Default for OnvifConfig {
    fn default() -> Self {
        Self {
            auth_policy: AuthPolicy::UsernameToken,
            media_preference: MediaPreference::Media2ThenMedia1,
            pull_point: PullPointConfig::default(),
            snapshot: SnapshotConfig::default(),
            xaddr_policy: XAddrPolicy::default(),
            max_provisioning_state_entries: 4096,
            parser: ParserLimits::default(),
        }
    }
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

    /// Sets the XAddr SSRF policy.
    pub fn with_xaddr_policy(mut self, xaddr_policy: XAddrPolicy) -> Self {
        self.xaddr_policy = xaddr_policy;
        self
    }

    /// Sets the maximum number of in-flight provisioning states.
    pub fn with_max_provisioning_state_entries(mut self, max: usize) -> Self {
        self.max_provisioning_state_entries = max.max(1);
        self
    }
}

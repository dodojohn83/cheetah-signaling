//! Scheduler and registry configuration.

/// Configuration for the media node registry gRPC service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaRegistryConfig {
    /// When true, register/heartbeat/deregister require an mTLS peer identity
    /// extension that matches the claimed node id.
    pub require_mtls: bool,
    /// Default lease duration in milliseconds for new registrations.
    pub default_lease_ttl_ms: u64,
    /// Heartbeat timeout in milliseconds after which a node is considered stale.
    pub heartbeat_timeout_ms: u64,
    /// Allowed URI schemes for a media node control endpoint.
    pub allowed_endpoint_schemes: Vec<String>,
    /// Maximum length of a control endpoint URI in bytes.
    pub max_endpoint_uri_length: usize,
    /// Maximum length of any free-form string field supplied by a media node.
    pub max_string_field_length: usize,
    /// Maximum number of operations advertised per MediaCapability.
    pub max_capability_operations: usize,
    /// Maximum number of constraint key-value pairs per MediaCapability.
    pub max_capability_constraints: usize,
    /// When false, loopback, link-local and private network endpoints are rejected.
    pub allow_internal_endpoints: bool,
    /// Timeout for DNS resolution during endpoint validation.
    pub endpoint_dns_lookup_timeout_ms: u64,
    /// Maximum CPU load percentage a heartbeat may report (0-100).
    pub max_reported_load_percent: u64,
    /// Maximum session count a heartbeat may report.
    pub max_reported_session_count: u64,
}

impl Default for MediaRegistryConfig {
    fn default() -> Self {
        Self::production()
    }
}

impl MediaRegistryConfig {
    /// Returns a default production configuration.
    pub fn production() -> Self {
        Self {
            require_mtls: true,
            default_lease_ttl_ms: 30_000,
            heartbeat_timeout_ms: 60_000,
            allowed_endpoint_schemes: vec!["http".to_string(), "https".to_string()],
            max_endpoint_uri_length: 2048,
            max_string_field_length: 256,
            max_capability_operations: 64,
            max_capability_constraints: 64,
            allow_internal_endpoints: false,
            endpoint_dns_lookup_timeout_ms: 1_000,
            max_reported_load_percent: 100,
            max_reported_session_count: 100_000,
        }
    }

    /// Returns a configuration suitable for tests that use loopback endpoints.
    pub fn test() -> Self {
        Self {
            allow_internal_endpoints: true,
            endpoint_dns_lookup_timeout_ms: 100,
            ..Self::production()
        }
    }
}

/// Weights used by the least-loaded scheduler.
#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    /// Score weight for available session capacity.
    pub available_sessions_weight: f64,
    /// Score weight for bandwidth headroom.
    pub bandwidth_weight: f64,
    /// Score weight for CPU headroom.
    pub cpu_weight: f64,
    /// Score weight for matching the preferred zone.
    pub zone_affinity_weight: f64,
    /// Score weight for a stable random factor.
    pub stable_random_weight: f64,
    /// Maximum attempts to reserve capacity on a selected node.
    pub max_reserve_attempts: usize,
    /// Maximum number of nodes to score per scheduling request.
    pub max_candidates: usize,
    /// Maximum number of simultaneous reservations tracked by the scheduler.
    pub max_reservations: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            available_sessions_weight: 1.0,
            bandwidth_weight: 0.5,
            cpu_weight: 0.8,
            zone_affinity_weight: 2.0,
            stable_random_weight: 0.3,
            max_reserve_attempts: 3,
            max_candidates: 256,
            max_reservations: 100_000,
        }
    }
}

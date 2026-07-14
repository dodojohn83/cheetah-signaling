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
        }
    }
}

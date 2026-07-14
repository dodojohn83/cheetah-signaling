//! Media client configuration.

/// Configuration for the media control client.
#[derive(Clone, Debug)]
pub struct MediaClientConfig {
    /// Connect timeout for new gRPC connections.
    pub connect_timeout_ms: u64,
    /// Timeout for a single RPC attempt.
    pub request_timeout_ms: u64,
    /// Maximum number of retry attempts for retryable errors.
    pub max_retry_attempts: usize,
    /// Base delay between retries in milliseconds.
    pub retry_base_delay_ms: u64,
    /// Maximum delay between retries in milliseconds.
    pub retry_max_delay_ms: u64,
    /// Per-node concurrency limit.
    pub per_node_concurrency: usize,
    /// Consecutive failures before opening the circuit breaker.
    pub circuit_breaker_threshold: u32,
    /// Cooldown before allowing traffic through an open circuit breaker.
    pub circuit_breaker_cooldown_ms: u64,
}

impl Default for MediaClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 5_000,
            request_timeout_ms: 10_000,
            max_retry_attempts: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 5_000,
            per_node_concurrency: 16,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 30_000,
        }
    }
}

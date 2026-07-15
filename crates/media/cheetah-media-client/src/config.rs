//! Media client configuration.

/// Configuration for the media control client.
#[derive(Clone)]
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
    /// Maximum number of pooled endpoint connections.
    pub max_connections: usize,
    /// When false, plain `http://` endpoints are rejected.
    pub allow_insecure_http: bool,
    /// When false, loopback, link-local and private network endpoints are rejected.
    pub allow_internal_endpoints: bool,
    /// Optional PEM-encoded CA certificate for TLS verification.
    pub tls_ca_pem: Option<String>,
    /// Optional PEM-encoded client certificate for mTLS.
    pub tls_client_cert_pem: Option<String>,
    /// Optional PEM-encoded client private key for mTLS.
    pub tls_client_key_pem: Option<String>,
    /// Timeout for DNS resolution during endpoint validation.
    pub endpoint_dns_lookup_timeout_ms: u64,
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
            max_connections: 10_000,
            allow_insecure_http: false,
            allow_internal_endpoints: false,
            tls_ca_pem: None,
            tls_client_cert_pem: None,
            tls_client_key_pem: None,
            endpoint_dns_lookup_timeout_ms: 1_000,
        }
    }
}

impl MediaClientConfig {
    /// Returns a configuration suitable for tests that use loopback HTTP endpoints.
    pub fn test() -> Self {
        Self {
            allow_insecure_http: true,
            allow_internal_endpoints: true,
            endpoint_dns_lookup_timeout_ms: 100,
            ..Self::default()
        }
    }
}

impl std::fmt::Debug for MediaClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaClientConfig")
            .field("connect_timeout_ms", &self.connect_timeout_ms)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("max_retry_attempts", &self.max_retry_attempts)
            .field("retry_base_delay_ms", &self.retry_base_delay_ms)
            .field("retry_max_delay_ms", &self.retry_max_delay_ms)
            .field("per_node_concurrency", &self.per_node_concurrency)
            .field("circuit_breaker_threshold", &self.circuit_breaker_threshold)
            .field(
                "circuit_breaker_cooldown_ms",
                &self.circuit_breaker_cooldown_ms,
            )
            .field("max_connections", &self.max_connections)
            .field("allow_insecure_http", &self.allow_insecure_http)
            .field("allow_internal_endpoints", &self.allow_internal_endpoints)
            .field(
                "tls_ca_pem",
                &self.tls_ca_pem.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "tls_client_cert_pem",
                &self.tls_client_cert_pem.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "tls_client_key_pem",
                &self.tls_client_key_pem.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "endpoint_dns_lookup_timeout_ms",
                &self.endpoint_dns_lookup_timeout_ms,
            )
            .finish()
    }
}

//! Configuration model for Cheetah Signaling.
//!
//! The root [`SignalConfig`] is a plain data structure that can be loaded from
//! layered sources. Secret fields are stored as `SecretString` and are redacted
//! in `Debug` output.

use crate::error::{Result, SignalError, SignalErrorKind};
use crate::{DurationMs, NodeId};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::net::SocketAddr;

/// Maximum byte length of free-form compatibility profile string fields.
pub const MAX_COMPATIBILITY_FIELD_BYTES: usize = 512;
/// Maximum number of allowed CORS origins.
const MAX_CORS_ORIGINS: usize = 64;
/// Maximum byte length of a single CORS allowed origin.
const MAX_CORS_ORIGIN_BYTES: usize = 256;
/// Maximum number of async worker threads (used for shard metrics sizing).
const MAX_WORKER_THREADS: usize = 1024;
/// Maximum number of blocking threads.
const MAX_BLOCKING_THREADS: usize = 10_000;
/// Maximum per-actor mailbox queue depth.
const MAX_QUEUE_DEPTH: usize = 1_000_000;
/// Maximum idle blocking thread keep-alive in milliseconds (one hour).
const MAX_THREAD_KEEP_ALIVE_MS: i64 = 3_600_000;
/// Maximum HTTP request read timeout in milliseconds (five minutes).
///
/// Larger values can overflow the deadline timestamp used by the request
/// context, causing the handler to time out immediately instead of waiting.
const MAX_HTTP_READ_TIMEOUT_MS: i64 = 300_000;

/// Maximum byte length of `http.listen_addr`.
const MAX_HTTP_ADDR_BYTES: usize = 256;
/// Maximum byte length of HTTP TLS/secret references.
const MAX_HTTP_SECRET_REF_BYTES: usize = 256;
/// Maximum HTTP rate-limit requests per second.
const MAX_HTTP_RATE_LIMIT_REQUESTS_PER_SECOND: u32 = 1_000_000;
/// Maximum HTTP rate-limit burst capacity.
const MAX_HTTP_RATE_LIMIT_BURST: u32 = 1_000_000;
/// Serializes a `SecretString` as a redacted placeholder, preserving empty defaults.
///
/// Empty secrets are written as `""` so the default configuration round-trips
/// correctly. Non-empty secrets are redacted to avoid leaking sensitive values
/// in example or debug output.
fn serialize_secret_string<S: Serializer>(
    value: &SecretString,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    let exposed = value.expose_secret();
    if exposed.is_empty() {
        serializer.serialize_str("")
    } else {
        serializer.serialize_str("[REDACTED]")
    }
}

/// Deserializes a `SecretString` from a string.
fn deserialize_secret_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<SecretString, D::Error> {
    let value = String::deserialize(deserializer)?;
    Ok(SecretString::from(value))
}

/// Root configuration for the Cheetah Signaling process.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct SignalConfig {
    /// System level settings.
    pub system: SystemConfig,
    /// Async runtime settings.
    pub runtime: RuntimeConfig,
    /// HTTP API settings.
    pub http: HttpConfig,
    /// gRPC API settings.
    pub grpc: GrpcConfig,
    /// Storage backend settings.
    pub storage: StorageConfig,
    /// Messaging backend settings.
    pub messaging: MessagingConfig,
    /// Clustering settings.
    pub cluster: ClusterConfig,
    /// Media coordination settings.
    pub media: MediaConfig,
    /// Plugin runtime settings.
    pub plugins: PluginsConfig,
    /// GB28181 protocol settings.
    pub gb28181: Gb28181Config,
    /// ONVIF protocol settings.
    pub onvif: OnvifConfig,
    /// Security and authentication settings.
    pub security: SecurityConfig,
    /// Secret provider configuration.
    pub secret: SecretConfig,
    /// Observability settings.
    pub observability: ObservabilityConfig,
}

impl SignalConfig {
    /// Validates the configuration for consistency and allowed ranges.
    pub fn validate(&self) -> Result<()> {
        if self.runtime.worker_threads == 0 || self.runtime.worker_threads > MAX_WORKER_THREADS {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("runtime.worker_threads must be between 1 and {MAX_WORKER_THREADS}"),
            ));
        }
        if self.runtime.max_blocking_threads == 0
            || self.runtime.max_blocking_threads > MAX_BLOCKING_THREADS
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "runtime.max_blocking_threads must be between 1 and {MAX_BLOCKING_THREADS}"
                ),
            ));
        }
        let keep_alive_ms = self.runtime.thread_keep_alive_ms.as_millis();
        if keep_alive_ms <= 0 || keep_alive_ms > MAX_THREAD_KEEP_ALIVE_MS {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "runtime.thread_keep_alive_ms must be between 1 and {MAX_THREAD_KEEP_ALIVE_MS}"
                ),
            ));
        }
        if self.runtime.queue_depth == 0 || self.runtime.queue_depth > MAX_QUEUE_DEPTH {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("runtime.queue_depth must be between 1 and {MAX_QUEUE_DEPTH}"),
            ));
        }
        self.gb28181.validate()?;
        self.http.validate()?;
        if self.http.port == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "http.port must not be zero",
            ));
        }
        if self.grpc.port == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "grpc.port must not be zero",
            ));
        }
        if self.grpc.mtls_client_ca_ref.is_some()
            && (self.grpc.tls_cert_ref.is_none() || self.grpc.tls_key_ref.is_none())
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "grpc.mtls_client_ca_ref requires both grpc.tls_cert_ref and grpc.tls_key_ref",
            ));
        }
        if self.grpc.tls_cert_ref.is_some() != self.grpc.tls_key_ref.is_some() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "grpc.tls_cert_ref and grpc.tls_key_ref must both be set or both be unset",
            ));
        }
        let static_key = self.security.static_api_key.expose_secret();
        if !static_key.is_empty() && static_key.len() < 32 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "security.static_api_key must be at least 32 characters when configured",
            ));
        }
        let jwt_public_key = self.security.jwt_public_key_ref.expose_secret();
        if !jwt_public_key.is_empty() {
            if self.security.jwt_audience.is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "security.jwt_audience must be configured when jwt_public_key_ref is set",
                ));
            }
            if self.security.jwt_issuer.is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "security.jwt_issuer must be configured when jwt_public_key_ref is set",
                ));
            }
        }
        if !(0.0..=1.0).contains(&self.observability.diagnostic_sample_rate) {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "observability.diagnostic_sample_rate must be in [0.0, 1.0]",
            ));
        }
        if self.observability.diagnostic_sample_rate > 0.0
            && self.observability.diagnostic_max_duration_ms == 0
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "observability.diagnostic_max_duration_ms must be greater than zero when sampling is enabled",
            ));
        }
        if self.storage.max_connections == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "storage.max_connections must be greater than zero",
            ));
        }

        let inferred = self.infer_deployment_profile()?;
        match inferred {
            DeploymentProfile::Edge => {
                if self.storage.backend != StorageBackend::Sqlite {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        "edge profile requires storage.backend = \"sqlite\"",
                    ));
                }
                if self.messaging.backend != MessagingBackend::Local {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        "edge profile requires messaging.backend = \"local\"",
                    ));
                }
                if self.cluster.enabled {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        "edge profile requires cluster.enabled = false",
                    ));
                }
            }
            DeploymentProfile::Cluster => {
                if self.storage.backend != StorageBackend::Postgres {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        "cluster profile requires storage.backend = \"postgres\"",
                    ));
                }
                if self.messaging.backend != MessagingBackend::Nats {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        "cluster profile requires messaging.backend = \"nats\"",
                    ));
                }
                if !self.cluster.enabled {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        "cluster profile requires cluster.enabled = true",
                    ));
                }
                if self.grpc.tls_cert_ref.is_none()
                    || self.grpc.tls_key_ref.is_none()
                    || self.grpc.mtls_client_ca_ref.is_none()
                {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        "cluster profile requires grpc.tls_cert_ref, tls_key_ref and mtls_client_ca_ref",
                    ));
                }
            }
        }
        self.validate_gb28181_challenge_optional_policy(&inferred)?;
        self.system.validate()?;
        self.security.validate()?;
        self.storage.validate()?;
        self.media.validate()?;
        self.grpc.validate()?;
        self.messaging.validate()?;
        self.plugins.validate()?;
        self.observability.validate()?;
        self.cluster.validate()?;
        self.secret.validate()?;
        self.onvif.validate()?;
        Ok(())
    }

    /// Enforces the GB28181 insecure-startup policy for `challenge_optional`.
    ///
    /// Accepting REGISTER without a successful digest exchange disables the
    /// primary device authentication control, so it must never be enabled
    /// implicitly. It is permitted only when the operator has explicitly
    /// selected the development/edge profile (`system.profile = "edge"`):
    ///
    /// - the cluster/production profile rejects it outright;
    /// - an inferred (unset) profile rejects it, forcing an explicit opt-in.
    fn validate_gb28181_challenge_optional_policy(
        &self,
        inferred: &DeploymentProfile,
    ) -> Result<()> {
        if !self.gb28181.challenge_optional_requested() {
            return Ok(());
        }
        if *inferred == DeploymentProfile::Cluster {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "gb28181 challenge_optional=true is not permitted in the cluster \
                 profile; every REGISTER must complete digest authentication",
            ));
        }
        if self.system.profile != Some(DeploymentProfile::Edge) {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "gb28181 challenge_optional=true requires system.profile = \"edge\" \
                 to be set explicitly; it must not be enabled under an inferred profile",
            ));
        }
        Ok(())
    }

    /// Returns the effective deployment profile, inferring it from the other
    /// backend settings when the profile is not explicitly set.
    fn infer_deployment_profile(&self) -> Result<DeploymentProfile> {
        match &self.system.profile {
            Some(profile) => Ok(profile.clone()),
            None => {
                if self.storage.backend == StorageBackend::Postgres
                    && self.messaging.backend == MessagingBackend::Nats
                    && self.cluster.enabled
                {
                    Ok(DeploymentProfile::Cluster)
                } else if self.storage.backend == StorageBackend::Sqlite
                    && self.messaging.backend == MessagingBackend::Local
                    && !self.cluster.enabled
                {
                    Ok(DeploymentProfile::Edge)
                } else {
                    Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        "could not infer deployment profile from storage/messaging/cluster settings; set system.profile explicitly",
                    ))
                }
            }
        }
    }

    /// Generates a TOML example of the default configuration.
    pub fn example_toml() -> Result<String> {
        let example = Self::default();
        toml::to_string_pretty(&example).map_err(|e| {
            SignalError::new(
                SignalErrorKind::Internal,
                "failed to serialize example config",
            )
            .with_source(e)
        })
    }
}

/// Deployment profile for the signaling process.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeploymentProfile {
    /// Single-node edge deployment with SQLite and local bus.
    #[default]
    Edge,
    /// Clustered deployment with PostgreSQL, NATS and ownership.
    Cluster,
}

/// System level configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct SystemConfig {
    /// Human readable node name.
    pub node_name: String,
    /// Data directory for local state.
    pub data_dir: String,
    /// Log level filter.
    pub log_level: String,
    /// Optional node id for stable identity.
    pub node_id: Option<NodeId>,
    /// Deployment profile. If omitted, it is inferred from storage/messaging/cluster settings.
    pub profile: Option<DeploymentProfile>,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            node_name: String::new(),
            data_dir: String::new(),
            log_level: "info".to_string(),
            node_id: None,
            profile: None,
        }
    }
}

/// Runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Number of async worker threads.
    pub worker_threads: usize,
    /// Maximum number of blocking threads.
    pub max_blocking_threads: usize,
    /// Keep alive for idle blocking threads in milliseconds.
    pub thread_keep_alive_ms: DurationMs,
    /// Default bounded queue depth for per-actor mailboxes.
    pub queue_depth: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            worker_threads: 4,
            max_blocking_threads: 512,
            thread_keep_alive_ms: DurationMs::from_seconds(10),
            queue_depth: 1_024,
        }
    }
}

/// HTTP API configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    /// Bind address for the HTTP server.
    pub listen_addr: String,
    /// Port for the HTTP server.
    pub port: u16,
    /// Reference to the TLS certificate secret.
    pub tls_cert_ref: Option<String>,
    /// Reference to the TLS key secret.
    pub tls_key_ref: Option<String>,
    /// Request read timeout.
    pub read_timeout_ms: DurationMs,
    /// Allowed CORS origins. Empty disables cross-origin requests.
    pub cors_allowed_origins: Vec<String>,
    /// Rate limit allowed requests per second per (source, tenant, protocol, node).
    pub rate_limit_requests_per_second: u32,
    /// Rate limit burst capacity.
    pub rate_limit_burst: u32,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0".to_string(),
            port: 8_080,
            tls_cert_ref: None,
            tls_key_ref: None,
            read_timeout_ms: DurationMs::from_seconds(5),
            cors_allowed_origins: Vec::new(),
            rate_limit_requests_per_second: 100,
            rate_limit_burst: 200,
        }
    }
}

impl HttpConfig {
    /// Validates HTTP-specific configuration.
    pub fn validate(&self) -> Result<()> {
        if self.listen_addr.len() > MAX_HTTP_ADDR_BYTES {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("http.listen_addr exceeds {MAX_HTTP_ADDR_BYTES} bytes"),
            ));
        }
        if let Some(ref r) = self.tls_cert_ref
            && r.len() > MAX_HTTP_SECRET_REF_BYTES
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("http.tls_cert_ref exceeds {MAX_HTTP_SECRET_REF_BYTES} bytes"),
            ));
        }
        if let Some(ref r) = self.tls_key_ref
            && r.len() > MAX_HTTP_SECRET_REF_BYTES
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("http.tls_key_ref exceeds {MAX_HTTP_SECRET_REF_BYTES} bytes"),
            ));
        }
        let timeout_ms = self.read_timeout_ms.as_millis();
        if timeout_ms <= 0 || timeout_ms > MAX_HTTP_READ_TIMEOUT_MS {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("http.read_timeout_ms must be between 1 and {MAX_HTTP_READ_TIMEOUT_MS}"),
            ));
        }
        if self.rate_limit_requests_per_second > MAX_HTTP_RATE_LIMIT_REQUESTS_PER_SECOND {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "http.rate_limit_requests_per_second must not exceed {MAX_HTTP_RATE_LIMIT_REQUESTS_PER_SECOND}"
                ),
            ));
        }
        if self.rate_limit_burst > MAX_HTTP_RATE_LIMIT_BURST {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("http.rate_limit_burst must not exceed {MAX_HTTP_RATE_LIMIT_BURST}"),
            ));
        }
        if self.cors_allowed_origins.len() > MAX_CORS_ORIGINS {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("http.cors_allowed_origins must not exceed {MAX_CORS_ORIGINS} entries"),
            ));
        }
        for origin in &self.cors_allowed_origins {
            if origin == "*" {
                continue;
            }
            if origin.len() > MAX_CORS_ORIGIN_BYTES {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "http.cors_allowed_origins entry exceeds {MAX_CORS_ORIGIN_BYTES} bytes"
                    ),
                ));
            }
            if origin.is_empty() || !origin.bytes().all(is_valid_header_value_byte) {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "http.cors_allowed_origins entry contains invalid characters",
                ));
            }
        }
        Ok(())
    }
}

/// Returns true for bytes accepted in an HTTP header value by the `http` crate:
/// visible ASCII characters, space and tab.
fn is_valid_header_value_byte(b: u8) -> bool {
    b == b'\t' || (b' '..=b'~').contains(&b)
}

/// gRPC API configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct GrpcConfig {
    /// Bind address for the gRPC server.
    pub listen_addr: String,
    /// Port for the gRPC server.
    pub port: u16,
    /// Reference to the TLS certificate secret.
    pub tls_cert_ref: Option<String>,
    /// Reference to the TLS key secret.
    pub tls_key_ref: Option<String>,
    /// Reference to the mTLS client CA certificate secret.
    /// When set, the gRPC server requires a client certificate.
    pub mtls_client_ca_ref: Option<String>,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0".to_string(),
            port: 50_051,
            tls_cert_ref: None,
            tls_key_ref: None,
            mtls_client_ca_ref: None,
        }
    }
}

/// Storage backend configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Selected storage backend.
    pub backend: StorageBackend,
    /// Path for the SQLite database.
    pub sqlite_path: String,
    /// Connection URL for PostgreSQL.
    #[serde(
        serialize_with = "serialize_secret_string",
        deserialize_with = "deserialize_secret_string"
    )]
    pub postgres_url: SecretString,
    /// Secret reference for the PostgreSQL URL. When set, takes precedence over `postgres_url`.
    pub postgres_url_ref: Option<String>,
    /// Maximum connection pool size.
    pub max_connections: u32,
    /// Connection acquisition timeout.
    pub connection_timeout_ms: DurationMs,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::Sqlite,
            sqlite_path: "/var/lib/cheetah/cheetah.db".to_string(),
            postgres_url: SecretString::default(),
            postgres_url_ref: None,
            max_connections: 10,
            connection_timeout_ms: DurationMs::from_seconds(5),
        }
    }
}

/// Supported storage backends.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StorageBackend {
    /// SQLite embedded database.
    #[default]
    Sqlite,
    /// PostgreSQL database.
    Postgres,
}

/// Messaging backend configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct MessagingConfig {
    /// Selected messaging backend.
    pub backend: MessagingBackend,
    /// NATS server URL.
    pub nats_url: String,
    /// Secret reference for the NATS URL. When set, takes precedence over `nats_url`.
    pub nats_url_ref: Option<String>,
    /// JetStream domain.
    pub jetstream_domain: String,
    /// Maximum pending messages per consumer.
    pub max_pending: usize,
}

/// Supported messaging backends.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessagingBackend {
    /// In process local bus.
    #[default]
    Local,
    /// NATS Core/JetStream.
    Nats,
}

/// Cluster configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ClusterConfig {
    /// Whether clustering is enabled.
    pub enabled: bool,
    /// Lease TTL for ownership in milliseconds.
    pub lease_ttl_ms: DurationMs,
    /// Heartbeat interval for cluster members.
    pub heartbeat_interval_ms: DurationMs,
}

/// Whether at least one alive media node is required for process readiness.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaReadinessPolicy {
    /// Media nodes are optional; API readiness does not depend on them.
    #[default]
    Optional,
    /// At least one media node with a valid lease is required for readiness.
    Required,
}

/// Media coordination configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct MediaConfig {
    /// Default selector for media nodes.
    pub default_media_node_selector: String,
    /// Maximum concurrent sessions per device.
    pub max_sessions_per_device: u32,
    /// Default timeout for media invitations.
    pub default_invite_timeout_ms: DurationMs,
    /// Interval between periodic media reconciliations.
    pub periodic_reconcile_interval_ms: DurationMs,
    /// Grace period before a NeedsVerification binding is escalated to migration
    /// or failure (milliseconds).
    pub needs_verification_grace_ms: DurationMs,
    /// Whether readiness requires an alive media node.
    pub readiness_policy: MediaReadinessPolicy,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            default_media_node_selector: "round-robin".to_string(),
            max_sessions_per_device: 4,
            default_invite_timeout_ms: DurationMs::from_seconds(30),
            periodic_reconcile_interval_ms: DurationMs::from_seconds(30),
            needs_verification_grace_ms: DurationMs::from_seconds(60),
            readiness_policy: MediaReadinessPolicy::Optional,
        }
    }
}

/// Plugin runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct PluginsConfig {
    /// Whether plugins are enabled.
    pub enabled: bool,
    /// Directory to load plugin binaries from.
    pub plugin_dir: String,
    /// Maximum plugin instances per node.
    pub max_plugin_instances: u32,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            plugin_dir: String::new(),
            max_plugin_instances: 8,
        }
    }
}

/// Upper bound for [`Gb28181Config::session_reaper_max_per_tick`]. Caps how
/// many expired sessions a single sweep buffers in memory before marking them
/// offline, so a misconfigured value cannot read an unbounded number of rows.
pub const SESSION_REAPER_MAX_PER_TICK_LIMIT: u32 = 1_000_000;
/// Upper bound for [`Gb28181Config::catalog_fragment_max_entries`].
const MAX_CATALOG_FRAGMENT_ENTRIES: u32 = 10_000;
/// Upper bound for [`Gb28181Config::catalog_fragment_max_items`].
const MAX_CATALOG_FRAGMENT_ITEMS: u32 = 100_000;
/// Upper bound for [`Gb28181Config::record_fragment_max_entries`].
const MAX_RECORD_FRAGMENT_ENTRIES: u32 = 10_000;
/// Upper bound for [`Gb28181Config::record_fragment_max_items`].
const MAX_RECORD_FRAGMENT_ITEMS: u32 = 100_000;

/// GB28181 protocol configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Gb28181Config {
    /// SIP domain.
    pub sip_domain: String,
    /// Local SIP port.
    pub sip_port: u16,
    /// Media stream timeout.
    pub media_stream_timeout_ms: DurationMs,
    /// Secret reference for the hex-encoded SIP digest secret used by this node.
    pub digest_secret_ref: Option<String>,
    /// Optional secret reference template for per-device SIP passwords.
    /// `{device_id}` is replaced with the GB device ID.
    pub device_password_ref: Option<String>,
    /// Optional default tenant UUID for SIP devices when no domain-to-tenant
    /// mapping is configured. When omitted, GB28181 events that cannot be
    /// attributed to a tenant are dropped.
    pub default_tenant_id: Option<String>,
    /// Maximum number of concurrent catalog fragment assemblies to keep in
    /// memory. Each assembly is keyed by the SIP sequence number of a catalog
    /// query response.
    pub catalog_fragment_max_entries: u32,
    /// Maximum number of catalog items that may be accumulated for a single
    /// (tenant, device, sequence number) before the partial assembly is dropped.
    pub catalog_fragment_max_items: u32,
    /// Maximum number of concurrent record-info fragment assemblies to keep in
    /// memory. Each assembly is keyed by the SIP sequence number of a record-info
    /// query response.
    pub record_fragment_max_entries: u32,
    /// Maximum number of record items that may be accumulated for a single
    /// (tenant, device, sequence number) before the partial assembly is dropped.
    pub record_fragment_max_items: u32,
    /// When true, accept REGISTER without successful digest authentication
    /// after issuing a challenge. Production deployments must leave this
    /// `false` (the default). Development profiles may enable it explicitly.
    pub challenge_optional: bool,
    /// Interval between protocol-session expiry reaper sweeps. Each sweep marks
    /// registrations whose `expiry_at` has passed offline. `0` disables the
    /// reaper.
    pub session_reaper_interval_ms: DurationMs,
    /// Page size used when the reaper scans expired sessions. Bounds the number
    /// of rows read per repository query.
    pub session_reaper_batch_size: u32,
    /// Maximum number of sessions the reaper marks offline in a single sweep.
    /// Bounds the work performed per tick so one node cannot monopolise the
    /// database. Must be in `1..=SESSION_REAPER_MAX_PER_TICK_LIMIT`.
    pub session_reaper_max_per_tick: u32,
    /// Compatibility profiles available to listeners and device bindings.
    ///
    /// Profiles are resolved at device binding time and the selected revision is
    /// pinned to the [`ProtocolSession`](cheetah_domain::ProtocolSession) so
    /// runtime changes do not alter in-flight dialogs.
    pub compatibility_profiles: Vec<Gb28181CompatibilityProfileConfig>,
    /// Explicit GB28181 listeners, each binding one or more sockets with its
    /// own realm/domain/tenant mapping.
    ///
    /// When non-empty this replaces the legacy single-listener fields
    /// (`sip_port`/`sip_domain`/`default_tenant_id`/...). Mixing legacy fields
    /// with `listeners` is rejected at validation time.
    pub listeners: Vec<Gb28181ListenerConfig>,
}

impl Default for Gb28181Config {
    fn default() -> Self {
        Self {
            sip_domain: String::new(),
            sip_port: 0,
            media_stream_timeout_ms: DurationMs::from_millis(0),
            digest_secret_ref: None,
            device_password_ref: None,
            default_tenant_id: None,
            catalog_fragment_max_entries: 1024,
            catalog_fragment_max_items: 8192,
            record_fragment_max_entries: 1024,
            record_fragment_max_items: 8192,
            challenge_optional: false,
            session_reaper_interval_ms: DurationMs::from_millis(30_000),
            session_reaper_batch_size: 256,
            session_reaper_max_per_tick: 4_096,
            compatibility_profiles: Vec::new(),
            listeners: Vec::new(),
        }
    }
}

/// Default GB28181 logical domain id used when the legacy `sip_domain` is unset.
///
/// Mirrors the historical single-listener default so devices provisioned against
/// the built-in domain keep the same digest realm after the migration to
/// explicit listeners.
pub const DEFAULT_GB28181_DOMAIN_ID: &str = "34020000002000000001";

/// A single GB28181 SIP listener with an explicit realm/domain/tenant mapping.
///
/// Each listener may bind a UDP and/or a TCP socket. The Request-URI/To domain
/// and the digest realm must uniquely resolve to a listener (and therefore a
/// tenant); ambiguous mappings are rejected so that a device can never be
/// silently attributed to the wrong tenant.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Gb28181ListenerConfig {
    /// Stable listener identifier, unique within the process. Used in logs and
    /// to disambiguate listeners that would otherwise share transport metadata.
    pub id: String,
    /// Tenant UUID that devices accepted by this listener are attributed to.
    pub tenant_id: String,
    /// The server's own GB28181 device/domain identifier for this listener.
    pub local_device_id: String,
    /// SIP realm advertised in digest challenges for this listener.
    pub realm: String,
    /// SIP domain (Request-URI/To host) that selects this listener.
    pub domain: String,
    /// UDP bind address, if this listener serves UDP.
    pub udp_bind: Option<SocketAddr>,
    /// TCP bind address, if this listener serves TCP.
    pub tcp_bind: Option<SocketAddr>,
    /// Secret reference for this listener's hex-encoded digest secret.
    pub digest_secret_ref: String,
    /// Optional secret reference template for per-device SIP passwords.
    /// `{device_id}` is replaced with the GB device ID.
    pub device_password_ref: Option<String>,
    /// When true, accept REGISTER without successful digest authentication
    /// after issuing a challenge. Production listeners must leave this `false`.
    pub challenge_optional: bool,
    /// Optional compatibility profile id applied to devices accepted by this
    /// listener. The id must match one of the profiles declared in
    /// [`Gb28181Config::compatibility_profiles`].
    pub compatibility_profile: Option<String>,
}

/// GB28181 compatibility profile configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Gb28181CompatibilityProfileConfig {
    /// Stable profile identifier, unique within the GB28181 configuration.
    pub id: String,
    /// GB/T 28181 standard version, e.g. `2011` or `2016`.
    pub standard_version: Option<String>,
    /// Device manufacturer name.
    pub manufacturer: Option<String>,
    /// Device model name.
    pub model: Option<String>,
    /// Device firmware version.
    pub firmware: Option<String>,
    /// Controlled capability names (snake_case) enabled by this profile.
    pub capabilities: Vec<String>,
    /// Path or URL to the provenance fixture that justifies this profile.
    pub evidence_ref: Option<String>,
    /// Profile revision, used to detect profile changes and pin sessions.
    pub revision: u32,
    /// Controlled media-negotiation overrides (SDP/broadcast/MediaStatus).
    pub overrides: Gb28181CompatibilityOverridesConfig,
}

/// Maximum number of entries in any single compatibility override list.
pub const MAX_COMPATIBILITY_OVERRIDE_ENTRIES: usize = 64;

/// Maximum byte length of an individual compatibility override list entry.
pub const MAX_COMPATIBILITY_OVERRIDE_ENTRY_BYTES: usize = 64;
/// Maximum number of GB28181 listeners.
const MAX_GB28181_LISTENERS: usize = 256;
/// Maximum byte length of a GB28181 listener identifier.
const MAX_GB28181_LISTENER_ID_BYTES: usize = 64;
/// Maximum byte length of a GB28181 listener realm.
const MAX_GB28181_LISTENER_REALM_BYTES: usize = 64;
/// Maximum byte length of a GB28181 listener domain.
const MAX_GB28181_LISTENER_DOMAIN_BYTES: usize = 64;
/// Maximum byte length of a GB28181 listener local device id.
const MAX_GB28181_LISTENER_LOCAL_DEVICE_ID_BYTES: usize = 64;
/// Maximum byte length of a GB28181 listener tenant id string.
const MAX_GB28181_LISTENER_TENANT_ID_BYTES: usize = 64;
/// Maximum byte length of a GB28181 listener secret reference.
const MAX_GB28181_LISTENER_SECRET_REF_BYTES: usize = 256;

/// Controlled media-negotiation overrides for a GB28181 compatibility profile.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Gb28181CompatibilityOverridesConfig {
    /// Extra RTP payload types (decimal strings) tolerated in device SDP answers.
    pub sdp_allowed_payload_types: Vec<String>,
    /// Extra vendor `a=` attribute names tolerated in device SDP answers.
    pub sdp_allowed_attribute_names: Vec<String>,
    /// Broadcast/talk media connection address source (`media_node` or
    /// `signaling_host`). `None` keeps the default `media_node` behaviour.
    pub broadcast_address_source: Option<String>,
    /// Vendor `MediaStatus` `NotifyType` values normalised to the stopped
    /// outcome in addition to the canonical `121`.
    pub media_status_stopped_codes: Vec<String>,
}

impl Gb28181CompatibilityOverridesConfig {
    /// Returns `true` when no override is configured.
    pub fn is_empty(&self) -> bool {
        self.sdp_allowed_payload_types.is_empty()
            && self.sdp_allowed_attribute_names.is_empty()
            && self.broadcast_address_source.is_none()
            && self.media_status_stopped_codes.is_empty()
    }

    fn validate(&self, profile_id: &str) -> Result<()> {
        for (field, entries) in [
            ("sdp_allowed_payload_types", &self.sdp_allowed_payload_types),
            (
                "sdp_allowed_attribute_names",
                &self.sdp_allowed_attribute_names,
            ),
            (
                "media_status_stopped_codes",
                &self.media_status_stopped_codes,
            ),
        ] {
            if entries.len() > MAX_COMPATIBILITY_OVERRIDE_ENTRIES {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 compatibility profile '{profile_id}' override '{field}' \
                         must not exceed {MAX_COMPATIBILITY_OVERRIDE_ENTRIES} entries"
                    ),
                ));
            }
            for entry in entries {
                if entry.trim().is_empty() {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        format!(
                            "gb28181 compatibility profile '{profile_id}' override '{field}' \
                             entries must not be empty"
                        ),
                    ));
                }
                if entry.len() > MAX_COMPATIBILITY_OVERRIDE_ENTRY_BYTES {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        format!(
                            "gb28181 compatibility profile '{profile_id}' override '{field}' \
                             entry exceeds maximum length"
                        ),
                    ));
                }
            }
        }
        if let Some(source) = &self.broadcast_address_source
            && !matches!(source.as_str(), "media_node" | "signaling_host")
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "gb28181 compatibility profile '{profile_id}' broadcast_address_source \
                     must be 'media_node' or 'signaling_host', got '{source}'"
                ),
            ));
        }
        Ok(())
    }
}

impl Gb28181Config {
    /// Returns true when any legacy single-listener field is set to a
    /// non-default value.
    fn has_legacy_listener(&self) -> bool {
        self.sip_port != 0
            || !self.sip_domain.is_empty()
            || self.digest_secret_ref.is_some()
            || self.device_password_ref.is_some()
            || self.default_tenant_id.is_some()
            || self.challenge_optional
    }

    /// Returns true when the insecure `challenge_optional` policy is requested
    /// by the legacy single-listener field or by any explicit listener.
    ///
    /// This is the trigger for the startup profile policy: unauthenticated
    /// REGISTER is only permitted under an explicit development/edge profile.
    pub fn challenge_optional_requested(&self) -> bool {
        self.challenge_optional || self.listeners.iter().any(|l| l.challenge_optional)
    }

    /// Validates GB28181 listener configuration.
    ///
    /// Enforces that legacy and explicit listener configuration are not mixed,
    /// that every explicit listener is complete, and that listener ids, domains,
    /// realms and bind addresses are unambiguous.
    pub fn validate(&self) -> Result<()> {
        if self.catalog_fragment_max_entries == 0
            || self.catalog_fragment_max_entries > MAX_CATALOG_FRAGMENT_ENTRIES
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "gb28181.catalog_fragment_max_entries must be between 1 and {MAX_CATALOG_FRAGMENT_ENTRIES}"
                ),
            ));
        }
        if self.catalog_fragment_max_items == 0
            || self.catalog_fragment_max_items > MAX_CATALOG_FRAGMENT_ITEMS
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "gb28181.catalog_fragment_max_items must be between 1 and {MAX_CATALOG_FRAGMENT_ITEMS}"
                ),
            ));
        }
        if self.record_fragment_max_entries == 0
            || self.record_fragment_max_entries > MAX_RECORD_FRAGMENT_ENTRIES
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "gb28181.record_fragment_max_entries must be between 1 and {MAX_RECORD_FRAGMENT_ENTRIES}"
                ),
            ));
        }
        if self.record_fragment_max_items == 0
            || self.record_fragment_max_items > MAX_RECORD_FRAGMENT_ITEMS
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "gb28181.record_fragment_max_items must be between 1 and {MAX_RECORD_FRAGMENT_ITEMS}"
                ),
            ));
        }
        if self.session_reaper_batch_size == 0
            || self.session_reaper_batch_size > crate::MAX_PAGE_SIZE
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "gb28181.session_reaper_batch_size must be between 1 and {}",
                    crate::MAX_PAGE_SIZE
                ),
            ));
        }
        if self.session_reaper_max_per_tick == 0
            || self.session_reaper_max_per_tick > SESSION_REAPER_MAX_PER_TICK_LIMIT
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!(
                    "gb28181.session_reaper_max_per_tick must be between 1 and {SESSION_REAPER_MAX_PER_TICK_LIMIT}"
                ),
            ));
        }

        let mut profile_ids = std::collections::HashSet::new();
        for profile in &self.compatibility_profiles {
            if profile.id.trim().is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "gb28181.compatibility_profiles[].id must not be empty",
                ));
            }
            for (name, value) in [
                ("id", Some(profile.id.as_str())),
                ("standard_version", profile.standard_version.as_deref()),
                ("manufacturer", profile.manufacturer.as_deref()),
                ("model", profile.model.as_deref()),
                ("firmware", profile.firmware.as_deref()),
                ("evidence_ref", profile.evidence_ref.as_deref()),
            ] {
                if let Some(v) = value
                    && v.len() > MAX_COMPATIBILITY_FIELD_BYTES
                {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        format!("gb28181.compatibility_profiles[].{name} exceeds maximum length"),
                    ));
                }
            }
            if profile.capabilities.len() > 64 {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "gb28181.compatibility_profiles[].capabilities must not exceed 64 entries",
                ));
            }
            profile.overrides.validate(&profile.id)?;
            if !profile_ids.insert(profile.id.as_str()) {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 compatibility profile id '{}' is duplicated",
                        profile.id
                    ),
                ));
            }
        }

        if self.listeners.is_empty() {
            return Ok(());
        }

        if self.has_legacy_listener() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "gb28181.listeners cannot be combined with the legacy \
                 sip_port/sip_domain/default_tenant_id/digest_secret_ref/\
                 device_password_ref/challenge_optional settings; migrate the \
                 legacy fields into a listener entry",
            ));
        }
        if self.listeners.len() > MAX_GB28181_LISTENERS {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("gb28181.listeners must not exceed {MAX_GB28181_LISTENERS} entries"),
            ));
        }

        let mut ids = std::collections::HashSet::new();
        let mut domains = std::collections::HashSet::new();
        let mut realms = std::collections::HashSet::new();
        let mut udp_binds = std::collections::HashSet::new();
        let mut tcp_binds = std::collections::HashSet::new();

        for listener in &self.listeners {
            if listener.id.trim().is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "gb28181.listeners[].id must not be empty",
                ));
            }
            if listener.id.len() > MAX_GB28181_LISTENER_ID_BYTES {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "gb28181.listeners[].id exceeds maximum length",
                ));
            }
            if listener.domain.trim().is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!("gb28181 listener '{}' requires a domain", listener.id),
                ));
            }
            if listener.domain.len() > MAX_GB28181_LISTENER_DOMAIN_BYTES {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' domain exceeds maximum length",
                        listener.id
                    ),
                ));
            }
            if listener.realm.trim().is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!("gb28181 listener '{}' requires a realm", listener.id),
                ));
            }
            if listener.realm.len() > MAX_GB28181_LISTENER_REALM_BYTES {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' realm exceeds maximum length",
                        listener.id
                    ),
                ));
            }
            if listener.local_device_id.trim().is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' requires a local_device_id",
                        listener.id
                    ),
                ));
            }
            if listener.local_device_id.len() > MAX_GB28181_LISTENER_LOCAL_DEVICE_ID_BYTES {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' local_device_id exceeds maximum length",
                        listener.id
                    ),
                ));
            }
            if listener.tenant_id.trim().is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' requires a tenant_id; cluster \
                         listeners must not rely on an implicit default tenant",
                        listener.id
                    ),
                ));
            }
            if listener.tenant_id.len() > MAX_GB28181_LISTENER_TENANT_ID_BYTES {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' tenant_id exceeds maximum length",
                        listener.id
                    ),
                ));
            }
            if listener.digest_secret_ref.trim().is_empty() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' requires a digest_secret_ref",
                        listener.id
                    ),
                ));
            }
            if listener.digest_secret_ref.len() > MAX_GB28181_LISTENER_SECRET_REF_BYTES {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' digest_secret_ref exceeds maximum length",
                        listener.id
                    ),
                ));
            }
            if listener.udp_bind.is_none() && listener.tcp_bind.is_none() {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' must bind at least one of udp_bind or tcp_bind",
                        listener.id
                    ),
                ));
            }
            if let Some(password_ref) = &listener.device_password_ref
                && password_ref.len() > MAX_GB28181_LISTENER_SECRET_REF_BYTES
            {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 listener '{}' device_password_ref exceeds maximum length",
                        listener.id
                    ),
                ));
            }
            if let Some(profile_id) = &listener.compatibility_profile {
                if profile_id.len() > MAX_COMPATIBILITY_FIELD_BYTES {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        format!(
                            "gb28181 listener '{}' compatibility_profile exceeds maximum length",
                            listener.id
                        ),
                    ));
                }
                if !profile_ids.contains(profile_id.as_str()) {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        format!(
                            "gb28181 listener '{}' references unknown compatibility profile '{}'",
                            listener.id, profile_id
                        ),
                    ));
                }
            }
            if !ids.insert(listener.id.as_str()) {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!("gb28181 listener id '{}' is duplicated", listener.id),
                ));
            }
            if !domains.insert(listener.domain.as_str()) {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 domain '{}' maps to more than one listener; a \
                         domain must resolve to exactly one tenant",
                        listener.domain
                    ),
                ));
            }
            if !realms.insert(listener.realm.as_str()) {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!(
                        "gb28181 realm '{}' maps to more than one listener; a \
                         realm must resolve to exactly one tenant",
                        listener.realm
                    ),
                ));
            }
            if let Some(addr) = listener.udp_bind
                && !udp_binds.insert(addr)
            {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!("gb28181 udp bind address {addr} is used by more than one listener"),
                ));
            }
            if let Some(addr) = listener.tcp_bind
                && !tcp_binds.insert(addr)
            {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!("gb28181 tcp bind address {addr} is used by more than one listener"),
                ));
            }
        }

        Ok(())
    }

    /// Resolves the effective set of listeners.
    ///
    /// When explicit [`listeners`](Self::listeners) are configured they are
    /// returned as-is and the `bool` is `false`. Otherwise, if the legacy
    /// single-listener fields request a listener (`sip_port > 0`) they are
    /// converted into a single synthetic listener and the `bool` is `true` to
    /// let callers emit a deprecation log. When neither is configured the list
    /// is empty.
    ///
    /// This never validates; call [`validate`](Self::validate) first.
    pub fn resolve_listeners(&self) -> (Vec<Gb28181ListenerConfig>, bool) {
        if !self.listeners.is_empty() {
            return (self.listeners.clone(), false);
        }
        if self.sip_port == 0 {
            return (Vec::new(), false);
        }
        let udp_bind = SocketAddr::from(([0, 0, 0, 0], self.sip_port));
        // Preserve the historical single-listener default: an unset SIP domain
        // resolved to DEFAULT_GB28181_DOMAIN_ID for both the domain id and the
        // digest realm, so devices provisioned against the built-in default can
        // still authenticate after the migration to explicit listeners.
        let domain = if self.sip_domain.is_empty() {
            DEFAULT_GB28181_DOMAIN_ID.to_string()
        } else {
            self.sip_domain.clone()
        };
        let listener = Gb28181ListenerConfig {
            id: "legacy".to_string(),
            tenant_id: self.default_tenant_id.clone().unwrap_or_default(),
            local_device_id: domain.clone(),
            realm: domain.clone(),
            domain,
            udp_bind: Some(udp_bind),
            tcp_bind: None,
            digest_secret_ref: self.digest_secret_ref.clone().unwrap_or_default(),
            device_password_ref: self.device_password_ref.clone(),
            challenge_optional: self.challenge_optional,
            compatibility_profile: None,
        };
        (vec![listener], true)
    }
}

/// ONVIF protocol configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct OnvifConfig {
    /// Whether ONVIF discovery and command processing is enabled.
    pub enabled: bool,
    /// Discovery timeout.
    pub discovery_timeout_ms: DurationMs,
    /// Probe timeout.
    pub probe_timeout_ms: DurationMs,
    /// HTTP connect timeout.
    pub connect_timeout_ms: DurationMs,
    /// HTTP request timeout (includes body download).
    pub request_timeout_ms: DurationMs,
    /// Maximum response body size in bytes.
    pub max_response_bytes: usize,
    /// Maximum concurrent HTTP requests per driver client.
    pub max_concurrent_requests: usize,
    /// Maximum concurrent ONVIF service calls to the same device endpoint.
    pub per_device_concurrency: usize,
    /// Maximum number of device endpoints whose concurrency semaphore is kept
    /// in memory. Idle entries are evicted when the map exceeds this limit.
    pub max_tracked_device_endpoints: usize,
    /// Whether to follow HTTP redirects (each hop re-checked against policy).
    pub follow_redirects: bool,
    /// Allowed URL schemes for discovered device XAddrs.
    pub allowed_schemes: Vec<String>,
    /// Allowed destination ports for discovered device XAddrs. Empty allows any.
    pub allowed_ports: Vec<u16>,
    /// Whether private (RFC 1918) addresses are allowed.
    pub allow_private: bool,
    /// Whether loopback addresses are allowed.
    pub allow_loopback: bool,
    /// Whether IPv4 link-local and IPv6 unicast link-local addresses are allowed.
    pub allow_link_local: bool,
    /// Whether `0.0.0.0` / `::` is allowed.
    pub allow_unspecified: bool,
    /// Whether domain-name hosts are allowed.
    pub allow_domain_names: bool,
    /// Multicast group for WS-Discovery Probe.
    pub discovery_multicast: SocketAddr,
    /// Local bind address for discovery sockets.
    pub discovery_bind: SocketAddr,
    /// Maximum XML body size accepted for parsing discovery datagrams.
    pub discovery_max_datagram_bytes: usize,
    /// Maximum XML element depth for discovery datagrams.
    pub discovery_max_xml_depth: usize,
    /// Maximum XML elements to visit while parsing a discovery datagram.
    pub discovery_max_xml_nodes: usize,
    /// Maximum matched devices returned from a single ProbeMatches.
    pub discovery_max_matches: usize,
    /// Per-source rate limit window in seconds.
    pub discovery_rate_window_seconds: u64,
    /// Maximum discovery datagrams per source IP within the window.
    pub discovery_rate_max_per_source: u32,
    /// Maximum distinct source IPs tracked by the discovery rate limiter.
    pub discovery_rate_max_sources: usize,
    /// Interval between discovery sweeps. Zero disables periodic discovery.
    pub discovery_interval_ms: DurationMs,
    /// Maximum concurrent device detail probes during a discovery sweep.
    pub max_concurrent_probes: u32,
    /// How long to cache ONVIF `GetCapabilities`/`GetServices` results per
    /// endpoint, in milliseconds. Zero disables caching.
    pub capability_ttl_ms: DurationMs,
    /// Maximum number of endpoints kept in the ONVIF capability cache.
    pub capability_cache_capacity: usize,
    /// Optional default tenant UUID for discovered ONVIF devices.
    pub default_tenant_id: Option<String>,
    /// Optional default username for ONVIF device authentication.
    pub default_username: Option<String>,
    /// Optional SecretProvider reference for the default ONVIF device password.
    pub default_credentials_ref: Option<String>,
}

impl Default for OnvifConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            discovery_timeout_ms: DurationMs::from_millis(3_000),
            probe_timeout_ms: DurationMs::from_millis(5_000),
            connect_timeout_ms: DurationMs::from_millis(5_000),
            request_timeout_ms: DurationMs::from_millis(15_000),
            max_response_bytes: 2 * 1024 * 1024,
            max_concurrent_requests: 32,
            per_device_concurrency: 2,
            max_tracked_device_endpoints: 1_024,
            follow_redirects: false,
            allowed_schemes: vec!["http".to_string(), "https".to_string()],
            allowed_ports: vec![80, 443],
            allow_private: false,
            allow_loopback: false,
            allow_link_local: false,
            allow_unspecified: false,
            allow_domain_names: false,
            discovery_multicast: SocketAddr::from(([239, 255, 255, 250], 3702)),
            discovery_bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            discovery_max_datagram_bytes: 65_536,
            discovery_max_xml_depth: 64,
            discovery_max_xml_nodes: 4_096,
            discovery_max_matches: 256,
            discovery_rate_window_seconds: 60,
            discovery_rate_max_per_source: 120,
            discovery_rate_max_sources: 1_024,
            discovery_interval_ms: DurationMs::from_millis(0),
            max_concurrent_probes: 8,
            capability_ttl_ms: DurationMs::from_millis(300_000),
            capability_cache_capacity: 1_024,
            default_tenant_id: None,
            default_username: None,
            default_credentials_ref: None,
        }
    }
}

/// Security configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct SecurityConfig {
    /// Reference to the JWT public key secret.
    #[serde(
        serialize_with = "serialize_secret_string",
        deserialize_with = "deserialize_secret_string"
    )]
    pub jwt_public_key_ref: SecretString,
    /// Expected JWT audiences. Empty disables audience validation.
    pub jwt_audience: Vec<String>,
    /// Expected JWT issuers. Empty disables issuer validation.
    pub jwt_issuer: Vec<String>,
    /// API key hash for service to service calls.
    #[serde(
        serialize_with = "serialize_secret_string",
        deserialize_with = "deserialize_secret_string"
    )]
    pub api_key_hash: SecretString,
    /// Static API key for edge-mode management token authentication.
    #[serde(
        serialize_with = "serialize_secret_string",
        deserialize_with = "deserialize_secret_string"
    )]
    pub static_api_key: SecretString,
    /// Token time to live in milliseconds.
    pub token_ttl_ms: DurationMs,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            jwt_public_key_ref: SecretString::default(),
            jwt_audience: Vec::new(),
            jwt_issuer: Vec::new(),
            api_key_hash: SecretString::default(),
            static_api_key: SecretString::default(),
            token_ttl_ms: DurationMs::from_seconds(3600),
        }
    }
}

/// Secret provider configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct SecretConfig {
    /// Environment variable prefix for the env-backed secret store.
    pub env_prefix: String,
    /// Optional directory to read file-backed secrets from.
    pub file_dir: Option<String>,
}

impl Default for SecretConfig {
    fn default() -> Self {
        Self {
            env_prefix: "CHEETAH_SECRET_".to_string(),
            file_dir: None,
        }
    }
}

/// Observability configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Bind address for metrics.
    pub metrics_bind_addr: String,
    /// Optional tracing collector endpoint.
    pub tracing_endpoint: Option<String>,
    /// Log format (json or compact).
    pub log_format: LogFormat,
    /// Whether raw protocol body logging is enabled. Defaults to false.
    pub protocol_body_logging: bool,
    /// Diagnostic sampling rate in the range [0.0, 1.0]. 0.0 disables sampling.
    pub diagnostic_sample_rate: f64,
    /// Maximum duration in milliseconds a diagnostic trace may run.
    pub diagnostic_max_duration_ms: u64,
    /// Maximum bytes of a protocol body that may be sampled.
    pub diagnostic_max_body_bytes: usize,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            metrics_bind_addr: String::new(),
            tracing_endpoint: None,
            log_format: LogFormat::Json,
            protocol_body_logging: false,
            diagnostic_sample_rate: 0.0,
            diagnostic_max_duration_ms: 30_000,
            diagnostic_max_body_bytes: 4096,
        }
    }
}

/// Maximum byte length of the log format configuration value.
const MAX_LOG_FORMAT_INPUT_BYTES: usize = 64;

/// Supported log output formats.
#[derive(Clone, Copy, Debug, Default, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// Line-delimited JSON (default).
    #[default]
    Json,
    /// Compact human-readable text for edge interactive mode.
    Compact,
}

impl<'de> Deserialize<'de> for LogFormat {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LogFormatVisitor;

        impl<'de> serde::de::Visitor<'de> for LogFormatVisitor {
            type Value = LogFormat;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a log format string like \"json\" or \"compact\"")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<LogFormat, E>
            where
                E: serde::de::Error,
            {
                if value.len() > MAX_LOG_FORMAT_INPUT_BYTES {
                    return Err(E::custom(format!(
                        "log format value exceeds {MAX_LOG_FORMAT_INPUT_BYTES} bytes"
                    )));
                }
                let trimmed = value.trim();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("json") {
                    Ok(LogFormat::Json)
                } else if trimmed.eq_ignore_ascii_case("compact") {
                    Ok(LogFormat::Compact)
                } else {
                    Err(E::unknown_variant(trimmed, &["json", "compact"]))
                }
            }
        }

        deserializer.deserialize_any(LogFormatVisitor)
    }
}

/// Source of configuration snapshots.
///
/// Implementations are responsible for layering defaults, files, environment
/// variables and secrets in the correct priority.
pub trait ConfigSource: Send + Sync {
    /// Returns a fully resolved, validated configuration snapshot.
    fn snapshot(&self) -> Result<SignalConfig>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod gb28181_listener_tests {
    use super::*;

    fn listener(id: &str, domain: &str, realm: &str, udp_port: u16) -> Gb28181ListenerConfig {
        Gb28181ListenerConfig {
            id: id.to_string(),
            tenant_id: "00000000-0000-0000-0000-000000000001".to_string(),
            local_device_id: "34020000002000000001".to_string(),
            realm: realm.to_string(),
            domain: domain.to_string(),
            udp_bind: Some(SocketAddr::from(([0, 0, 0, 0], udp_port))),
            tcp_bind: None,
            digest_secret_ref: "secret://digest".to_string(),
            device_password_ref: None,
            challenge_optional: false,
            compatibility_profile: None,
        }
    }

    #[test]
    fn empty_gb28181_config_is_valid() {
        let cfg = Gb28181Config::default();
        assert!(cfg.validate().is_ok());
        let (listeners, legacy) = cfg.resolve_listeners();
        assert!(listeners.is_empty());
        assert!(!legacy);
    }

    #[test]
    fn legacy_fields_resolve_to_single_listener_with_deprecation_flag() {
        let mut cfg = Gb28181Config {
            sip_domain: "3402000000".to_string(),
            sip_port: 5060,
            digest_secret_ref: Some("secret://digest".to_string()),
            default_tenant_id: Some("00000000-0000-0000-0000-000000000001".to_string()),
            ..Gb28181Config::default()
        };
        cfg.challenge_optional = false;
        assert!(cfg.validate().is_ok());
        let (listeners, legacy) = cfg.resolve_listeners();
        assert!(legacy);
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].domain, "3402000000");
        assert_eq!(listeners[0].udp_bind.unwrap().port(), 5060);
    }

    #[test]
    fn legacy_empty_sip_domain_defaults_realm_and_domain() {
        let cfg = Gb28181Config {
            sip_domain: String::new(),
            sip_port: 5060,
            digest_secret_ref: Some("secret://digest".to_string()),
            ..Gb28181Config::default()
        };
        assert!(cfg.validate().is_ok());
        let (listeners, legacy) = cfg.resolve_listeners();
        assert!(legacy);
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].domain, DEFAULT_GB28181_DOMAIN_ID);
        assert_eq!(listeners[0].realm, DEFAULT_GB28181_DOMAIN_ID);
        assert_eq!(listeners[0].local_device_id, DEFAULT_GB28181_DOMAIN_ID);
    }

    #[test]
    fn mixing_legacy_and_listeners_is_rejected() {
        let mut cfg = Gb28181Config {
            sip_port: 5060,
            ..Gb28181Config::default()
        };
        cfg.listeners
            .push(listener("a", "3402000000", "realm-a", 5060));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn duplicate_domain_is_ambiguous() {
        let mut cfg = Gb28181Config::default();
        cfg.listeners
            .push(listener("a", "3402000000", "realm-a", 5060));
        cfg.listeners
            .push(listener("b", "3402000000", "realm-b", 5061));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn duplicate_realm_is_ambiguous() {
        let mut cfg = Gb28181Config::default();
        cfg.listeners.push(listener("a", "domain-a", "realm", 5060));
        cfg.listeners.push(listener("b", "domain-b", "realm", 5061));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn duplicate_udp_bind_is_rejected() {
        let mut cfg = Gb28181Config::default();
        cfg.listeners
            .push(listener("a", "domain-a", "realm-a", 5060));
        cfg.listeners
            .push(listener("b", "domain-b", "realm-b", 5060));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn listener_without_any_bind_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("a", "domain-a", "realm-a", 5060);
        l.udp_bind = None;
        l.tcp_bind = None;
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn listener_without_tenant_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("a", "domain-a", "realm-a", 5060);
        l.tenant_id = String::new();
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    fn profile_with_overrides(
        overrides: Gb28181CompatibilityOverridesConfig,
    ) -> Gb28181CompatibilityProfileConfig {
        Gb28181CompatibilityProfileConfig {
            id: "p1".to_string(),
            overrides,
            ..Gb28181CompatibilityProfileConfig::default()
        }
    }

    #[test]
    fn compatibility_override_defaults_are_valid() {
        let mut cfg = Gb28181Config::default();
        cfg.compatibility_profiles.push(profile_with_overrides(
            Gb28181CompatibilityOverridesConfig::default(),
        ));
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn compatibility_override_accepts_known_broadcast_source() {
        let mut cfg = Gb28181Config::default();
        cfg.compatibility_profiles.push(profile_with_overrides(
            Gb28181CompatibilityOverridesConfig {
                broadcast_address_source: Some("signaling_host".to_string()),
                ..Default::default()
            },
        ));
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn compatibility_override_rejects_unknown_broadcast_source() {
        let mut cfg = Gb28181Config::default();
        cfg.compatibility_profiles.push(profile_with_overrides(
            Gb28181CompatibilityOverridesConfig {
                broadcast_address_source: Some("nonsense".to_string()),
                ..Default::default()
            },
        ));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn compatibility_override_rejects_empty_entry() {
        let mut cfg = Gb28181Config::default();
        cfg.compatibility_profiles.push(profile_with_overrides(
            Gb28181CompatibilityOverridesConfig {
                sdp_allowed_payload_types: vec!["  ".to_string()],
                ..Default::default()
            },
        ));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn compatibility_override_rejects_too_many_entries() {
        let mut cfg = Gb28181Config::default();
        cfg.compatibility_profiles.push(profile_with_overrides(
            Gb28181CompatibilityOverridesConfig {
                media_status_stopped_codes: (0..(MAX_COMPATIBILITY_OVERRIDE_ENTRIES + 1))
                    .map(|i| i.to_string())
                    .collect(),
                ..Default::default()
            },
        ));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn distinct_listeners_are_valid_and_returned_as_is() {
        let mut cfg = Gb28181Config::default();
        cfg.listeners
            .push(listener("a", "domain-a", "realm-a", 5060));
        cfg.listeners
            .push(listener("b", "domain-b", "realm-b", 5061));
        assert!(cfg.validate().is_ok());
        let (listeners, legacy) = cfg.resolve_listeners();
        assert!(!legacy);
        assert_eq!(listeners.len(), 2);
    }

    #[test]
    fn too_many_listeners_is_rejected() {
        let mut cfg = Gb28181Config::default();
        for i in 0..=MAX_GB28181_LISTENERS {
            let port = 5060 + (i % 65536) as u16;
            cfg.listeners.push(listener(
                &format!("listener-{i}"),
                &format!("domain-{i}"),
                &format!("realm-{i}"),
                port,
            ));
        }
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_listener_id_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("x", "domain-a", "realm-a", 5060);
        l.id = "x".repeat(MAX_GB28181_LISTENER_ID_BYTES + 1);
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_listener_domain_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("x", "domain-a", "realm-a", 5060);
        l.domain = "x".repeat(MAX_GB28181_LISTENER_DOMAIN_BYTES + 1);
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_listener_realm_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("x", "domain-a", "realm-a", 5060);
        l.realm = "x".repeat(MAX_GB28181_LISTENER_REALM_BYTES + 1);
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_listener_local_device_id_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("x", "domain-a", "realm-a", 5060);
        l.local_device_id = "x".repeat(MAX_GB28181_LISTENER_LOCAL_DEVICE_ID_BYTES + 1);
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_listener_tenant_id_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("x", "domain-a", "realm-a", 5060);
        l.tenant_id = "x".repeat(MAX_GB28181_LISTENER_TENANT_ID_BYTES + 1);
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_listener_digest_secret_ref_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("x", "domain-a", "realm-a", 5060);
        l.digest_secret_ref = "x".repeat(MAX_GB28181_LISTENER_SECRET_REF_BYTES + 1);
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_listener_device_password_ref_is_rejected() {
        let mut cfg = Gb28181Config::default();
        let mut l = listener("x", "domain-a", "realm-a", 5060);
        l.device_password_ref = Some("x".repeat(MAX_GB28181_LISTENER_SECRET_REF_BYTES + 1));
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_listener_compatibility_profile_is_rejected() {
        let mut cfg = Gb28181Config::default();
        cfg.compatibility_profiles
            .push(Gb28181CompatibilityProfileConfig {
                id: "p1".to_string(),
                ..Gb28181CompatibilityProfileConfig::default()
            });
        let mut l = listener("x", "domain-a", "realm-a", 5060);
        l.compatibility_profile = Some("x".repeat(MAX_COMPATIBILITY_FIELD_BYTES + 1));
        cfg.listeners.push(l);
        assert!(cfg.validate().is_err());
    }

    fn gb28181_config_with(
        catalog_fragment_max_entries: u32,
        catalog_fragment_max_items: u32,
        record_fragment_max_entries: u32,
        record_fragment_max_items: u32,
    ) -> Gb28181Config {
        Gb28181Config {
            catalog_fragment_max_entries,
            catalog_fragment_max_items,
            record_fragment_max_entries,
            record_fragment_max_items,
            ..Gb28181Config::default()
        }
    }

    #[test]
    fn catalog_fragment_max_entries_bounds() {
        assert!(gb28181_config_with(0, 1, 1, 1).validate().is_err());
        assert!(
            gb28181_config_with(MAX_CATALOG_FRAGMENT_ENTRIES + 1, 1, 1, 1)
                .validate()
                .is_err()
        );
        assert!(
            gb28181_config_with(MAX_CATALOG_FRAGMENT_ENTRIES, 1, 1, 1)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn catalog_fragment_max_items_bounds() {
        assert!(gb28181_config_with(1, 0, 1, 1).validate().is_err());
        assert!(
            gb28181_config_with(1, MAX_CATALOG_FRAGMENT_ITEMS + 1, 1, 1)
                .validate()
                .is_err()
        );
        assert!(
            gb28181_config_with(1, MAX_CATALOG_FRAGMENT_ITEMS, 1, 1)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn record_fragment_max_entries_bounds() {
        assert!(gb28181_config_with(1, 1, 0, 1).validate().is_err());
        assert!(
            gb28181_config_with(1, 1, MAX_RECORD_FRAGMENT_ENTRIES + 1, 1)
                .validate()
                .is_err()
        );
        assert!(
            gb28181_config_with(1, 1, MAX_RECORD_FRAGMENT_ENTRIES, 1)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn record_fragment_max_items_bounds() {
        assert!(gb28181_config_with(1, 1, 1, 0).validate().is_err());
        assert!(
            gb28181_config_with(1, 1, 1, MAX_RECORD_FRAGMENT_ITEMS + 1)
                .validate()
                .is_err()
        );
        assert!(
            gb28181_config_with(1, 1, 1, MAX_RECORD_FRAGMENT_ITEMS)
                .validate()
                .is_ok()
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod http_config_tests {
    use super::*;

    fn http_config_with_origins(origins: Vec<String>) -> HttpConfig {
        HttpConfig {
            cors_allowed_origins: origins,
            ..HttpConfig::default()
        }
    }

    fn http_config_with_timeout(timeout_ms: i64) -> HttpConfig {
        HttpConfig {
            read_timeout_ms: DurationMs::from_millis(timeout_ms),
            ..HttpConfig::default()
        }
    }

    #[test]
    fn default_http_config_is_valid() {
        assert!(HttpConfig::default().validate().is_ok());
    }

    #[test]
    fn read_timeout_zero_is_rejected() {
        assert!(http_config_with_timeout(0).validate().is_err());
    }

    #[test]
    fn negative_read_timeout_is_rejected() {
        assert!(http_config_with_timeout(-1).validate().is_err());
    }

    #[test]
    fn oversized_read_timeout_is_rejected() {
        assert!(
            http_config_with_timeout(MAX_HTTP_READ_TIMEOUT_MS + 1)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn max_read_timeout_is_accepted() {
        assert!(
            http_config_with_timeout(MAX_HTTP_READ_TIMEOUT_MS)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn wildcard_cors_origin_is_valid() {
        assert!(
            http_config_with_origins(vec!["*".to_string()])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn valid_cors_origin_is_accepted() {
        assert!(
            http_config_with_origins(vec!["https://example.com:8080".to_string()])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn empty_cors_origin_is_rejected() {
        assert!(
            http_config_with_origins(vec!["".to_string()])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn oversized_cors_origin_is_rejected() {
        assert!(
            http_config_with_origins(vec!["x".repeat(MAX_CORS_ORIGIN_BYTES + 1)])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn too_many_cors_origins_is_rejected() {
        let origins = (0..MAX_CORS_ORIGINS + 1)
            .map(|i| format!("https://example{i}.com"))
            .collect();
        assert!(http_config_with_origins(origins).validate().is_err());
    }

    #[test]
    fn control_character_in_cors_origin_is_rejected() {
        assert!(
            http_config_with_origins(vec!["https://evil\n.com".to_string()])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn non_ascii_byte_in_cors_origin_is_rejected() {
        assert!(
            http_config_with_origins(vec!["https://exämple.com".to_string()])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn oversized_listen_addr_is_rejected() {
        let cfg = HttpConfig {
            listen_addr: "x".repeat(MAX_HTTP_ADDR_BYTES + 1),
            ..HttpConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_tls_cert_ref_is_rejected() {
        let cfg = HttpConfig {
            tls_cert_ref: Some("x".repeat(MAX_HTTP_SECRET_REF_BYTES + 1)),
            ..HttpConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_rate_limit_requests_per_second_is_rejected() {
        let cfg = HttpConfig {
            rate_limit_requests_per_second: MAX_HTTP_RATE_LIMIT_REQUESTS_PER_SECOND + 1,
            ..HttpConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn oversized_rate_limit_burst_is_rejected() {
        let cfg = HttpConfig {
            rate_limit_burst: MAX_HTTP_RATE_LIMIT_BURST + 1,
            ..HttpConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn max_rate_limit_values_are_accepted() {
        let cfg = HttpConfig {
            rate_limit_requests_per_second: MAX_HTTP_RATE_LIMIT_REQUESTS_PER_SECOND,
            rate_limit_burst: MAX_HTTP_RATE_LIMIT_BURST,
            ..HttpConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod runtime_config_tests {
    use super::*;

    fn signal_config_with_runtime(runtime: RuntimeConfig) -> SignalConfig {
        SignalConfig {
            runtime,
            ..SignalConfig::default()
        }
    }

    #[test]
    fn default_runtime_config_is_valid() {
        assert!(SignalConfig::default().validate().is_ok());
    }

    #[test]
    fn worker_threads_at_limit_is_valid() {
        let cfg = RuntimeConfig {
            worker_threads: MAX_WORKER_THREADS,
            ..RuntimeConfig::default()
        };
        assert!(signal_config_with_runtime(cfg).validate().is_ok());
    }

    #[test]
    fn too_many_worker_threads_is_rejected() {
        let cfg = RuntimeConfig {
            worker_threads: MAX_WORKER_THREADS + 1,
            ..RuntimeConfig::default()
        };
        assert!(signal_config_with_runtime(cfg).validate().is_err());
    }

    #[test]
    fn too_many_blocking_threads_is_rejected() {
        let cfg = RuntimeConfig {
            max_blocking_threads: MAX_BLOCKING_THREADS + 1,
            ..RuntimeConfig::default()
        };
        assert!(signal_config_with_runtime(cfg).validate().is_err());
    }

    #[test]
    fn queue_depth_at_limit_is_valid() {
        let cfg = RuntimeConfig {
            queue_depth: MAX_QUEUE_DEPTH,
            ..RuntimeConfig::default()
        };
        assert!(signal_config_with_runtime(cfg).validate().is_ok());
    }

    #[test]
    fn oversized_queue_depth_is_rejected() {
        let cfg = RuntimeConfig {
            queue_depth: MAX_QUEUE_DEPTH + 1,
            ..RuntimeConfig::default()
        };
        assert!(signal_config_with_runtime(cfg).validate().is_err());
    }

    #[test]
    fn excessive_thread_keep_alive_is_rejected() {
        let cfg = RuntimeConfig {
            thread_keep_alive_ms: DurationMs::from_millis(MAX_THREAD_KEEP_ALIVE_MS + 1),
            ..RuntimeConfig::default()
        };
        assert!(signal_config_with_runtime(cfg).validate().is_err());
    }
}

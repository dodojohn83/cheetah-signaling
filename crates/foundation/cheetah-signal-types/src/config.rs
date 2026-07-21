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
        if self.runtime.worker_threads == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "runtime.worker_threads must be greater than zero",
            ));
        }
        if self.runtime.queue_depth == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "runtime.queue_depth must be greater than zero",
            ));
        }
        if self.gb28181.catalog_fragment_max_entries == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "gb28181.catalog_fragment_max_entries must be greater than zero",
            ));
        }
        if self.gb28181.catalog_fragment_max_items == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "gb28181.catalog_fragment_max_items must be greater than zero",
            ));
        }
        if self.onvif.enabled {
            if self.onvif.connect_timeout_ms.as_millis() <= 0 {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "onvif.connect_timeout_ms must be greater than zero",
                ));
            }
            if self.onvif.request_timeout_ms.as_millis() <= 0 {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "onvif.request_timeout_ms must be greater than zero",
                ));
            }
            if self.onvif.max_response_bytes == 0 {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "onvif.max_response_bytes must be greater than zero",
                ));
            }
            if self.onvif.max_concurrent_requests == 0 {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "onvif.max_concurrent_requests must be greater than zero",
                ));
            }
            if self.onvif.max_concurrent_probes == 0 {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "onvif.max_concurrent_probes must be greater than zero",
                ));
            }
            if !self.onvif.allowed_schemes.is_empty()
                && !self.onvif.allowed_schemes.iter().all(|s| !s.is_empty())
            {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "onvif.allowed_schemes must not contain empty entries",
                ));
            }
            if self.onvif.discovery_interval_ms.as_millis() < 0 {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "onvif.discovery_interval_ms must not be negative",
                ));
            }
        }
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
        if self.http.read_timeout_ms.as_millis() <= 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "http.read_timeout_ms must be greater than zero",
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
        if self.observability.diagnostic_sample_rate < 0.0
            || self.observability.diagnostic_sample_rate > 1.0
        {
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
        if self.media.default_invite_timeout_ms.as_millis() <= 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "media.default_invite_timeout_ms must be greater than zero",
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
    /// Whether readiness requires an alive media node.
    pub readiness_policy: MediaReadinessPolicy,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            default_media_node_selector: "round-robin".to_string(),
            max_sessions_per_device: 4,
            default_invite_timeout_ms: DurationMs::from_seconds(30),
            readiness_policy: MediaReadinessPolicy::Optional,
        }
    }
}

/// Plugin runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
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
    /// database.
    pub session_reaper_max_per_tick: u32,
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
            challenge_optional: false,
            session_reaper_interval_ms: DurationMs::from_millis(30_000),
            session_reaper_batch_size: 256,
            session_reaper_max_per_tick: 4_096,
        }
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
    /// Optional default tenant UUID for discovered ONVIF devices.
    pub default_tenant_id: Option<String>,
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
            default_tenant_id: None,
        }
    }
}

/// Security configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
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
                let normalized = value.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "" | "json" => Ok(LogFormat::Json),
                    "compact" => Ok(LogFormat::Compact),
                    other => Err(E::unknown_variant(other, &["json", "compact"])),
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

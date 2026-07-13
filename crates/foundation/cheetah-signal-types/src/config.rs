//! Configuration model for Cheetah Signaling.
//!
//! The root [`SignalConfig`] is a plain data structure that can be loaded from
//! layered sources. Secret fields are stored as `SecretString` and are redacted
//! in `Debug` output.

use crate::error::{Result, SignalError, SignalErrorKind};
use crate::{DurationMs, NodeId};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serializes a `SecretString` as a redacted placeholder.
fn serialize_secret_string<S: Serializer>(
    value: &SecretString,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(value.expose_secret())
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
        Ok(())
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

/// System level configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SystemConfig {
    /// Human readable node name.
    pub node_name: String,
    /// Data directory for local state.
    pub data_dir: String,
    /// Log level filter.
    pub log_level: String,
    /// Optional node id for stable identity.
    pub node_id: Option<NodeId>,
}

/// Runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
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
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0".to_string(),
            port: 8_080,
            tls_cert_ref: None,
            tls_key_ref: None,
            read_timeout_ms: DurationMs::from_seconds(5),
        }
    }
}

/// gRPC API configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GrpcConfig {
    /// Bind address for the gRPC server.
    pub listen_addr: String,
    /// Port for the gRPC server.
    pub port: u16,
    /// Reference to the TLS certificate secret.
    pub tls_cert_ref: Option<String>,
    /// Reference to the TLS key secret.
    pub tls_key_ref: Option<String>,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0".to_string(),
            port: 50_051,
            tls_cert_ref: None,
            tls_key_ref: None,
        }
    }
}

/// Storage backend configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
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
pub struct MessagingConfig {
    /// Selected messaging backend.
    pub backend: MessagingBackend,
    /// NATS server URL.
    pub nats_url: String,
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
pub struct ClusterConfig {
    /// Whether clustering is enabled.
    pub enabled: bool,
    /// Lease TTL for ownership in milliseconds.
    pub lease_ttl_ms: DurationMs,
    /// Heartbeat interval for cluster members.
    pub heartbeat_interval_ms: DurationMs,
}

/// Media coordination configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaConfig {
    /// Default selector for media nodes.
    pub default_media_node_selector: String,
    /// Maximum concurrent sessions per device.
    pub max_sessions_per_device: u32,
    /// Default timeout for media invitations.
    pub default_invite_timeout_ms: DurationMs,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            default_media_node_selector: "round-robin".to_string(),
            max_sessions_per_device: 4,
            default_invite_timeout_ms: DurationMs::from_seconds(30),
        }
    }
}

/// Plugin runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PluginsConfig {
    /// Whether plugins are enabled.
    pub enabled: bool,
    /// Directory to load plugin binaries from.
    pub plugin_dir: String,
    /// Maximum plugin instances per node.
    pub max_plugin_instances: u32,
}

/// GB28181 protocol configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Gb28181Config {
    /// SIP domain.
    pub sip_domain: String,
    /// Local SIP port.
    pub sip_port: u16,
    /// Media stream timeout.
    pub media_stream_timeout_ms: DurationMs,
}

/// ONVIF protocol configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OnvifConfig {
    /// Discovery timeout.
    pub discovery_timeout_ms: DurationMs,
    /// Probe timeout.
    pub probe_timeout_ms: DurationMs,
}

/// Security configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SecurityConfig {
    /// Reference to the JWT public key secret.
    #[serde(
        serialize_with = "serialize_secret_string",
        deserialize_with = "deserialize_secret_string"
    )]
    pub jwt_public_key_ref: SecretString,
    /// API key hash for service to service calls.
    #[serde(
        serialize_with = "serialize_secret_string",
        deserialize_with = "deserialize_secret_string"
    )]
    pub api_key_hash: SecretString,
    /// Token time to live in milliseconds.
    pub token_ttl_ms: DurationMs,
}

/// Observability configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ObservabilityConfig {
    /// Bind address for metrics.
    pub metrics_bind_addr: String,
    /// Optional tracing collector endpoint.
    pub tracing_endpoint: Option<String>,
    /// Log format.
    pub log_format: String,
}

/// Source of configuration snapshots.
///
/// Implementations are responsible for layering defaults, files, environment
/// variables and secrets in the correct priority.
pub trait ConfigSource: Send + Sync {
    /// Returns a fully resolved, validated configuration snapshot.
    fn snapshot(&self) -> Result<SignalConfig>;
}

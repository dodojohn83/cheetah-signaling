//! Length and count bounds for configuration string/list fields.
//!
//! This module adds `validate` methods to the child config structs defined in
//! [`crate::config`]. It is kept separate so the main `config.rs` file does not
//! grow even larger with repetitive validation code.

use crate::config::*;
use crate::error::{Result, SignalError, SignalErrorKind};
use secrecy::ExposeSecret;

const MAX_SYSTEM_NODE_NAME_BYTES: usize = 256;
const MAX_SYSTEM_DATA_DIR_BYTES: usize = 4096;
const MAX_SYSTEM_LOG_LEVEL_BYTES: usize = 256;

const MAX_SECURITY_SECRET_REF_BYTES: usize = 256;
const MAX_STATIC_API_KEY_BYTES: usize = 4096;
const MAX_JWT_ENTRIES: usize = 16;
const MAX_JWT_STRING_BYTES: usize = 256;

const MAX_STORAGE_PATH_BYTES: usize = 4096;
const MAX_POSTGRES_URL_BYTES: usize = 4096;
const MAX_STORAGE_SECRET_REF_BYTES: usize = 256;
const MAX_STORAGE_CONNECTIONS: u32 = 10_000;
const MAX_STORAGE_CONNECTION_TIMEOUT_MS: i64 = 300_000; // 5 minutes

const MAX_MEDIA_NODE_SELECTOR_BYTES: usize = 256;

const MAX_MEDIA_INVITE_TIMEOUT_MS: i64 = 24 * 60 * 60 * 1_000; // 1 day
const MAX_MEDIA_RECONCILE_INTERVAL_MS: i64 = 24 * 60 * 60 * 1_000; // 1 day
const MAX_MEDIA_VERIFICATION_GRACE_MS: i64 = 30 * 24 * 60 * 60 * 1_000; // 30 days
const MAX_MEDIA_SESSIONS_PER_DEVICE: u32 = 1_000;

const MAX_GRPC_ADDR_BYTES: usize = 256;
const MAX_GRPC_SECRET_REF_BYTES: usize = 256;

const MAX_MESSAGING_URL_BYTES: usize = 4096;
const MAX_MESSAGING_SECRET_REF_BYTES: usize = 256;
const MAX_MESSAGING_DOMAIN_BYTES: usize = 256;
const MAX_MESSAGING_PENDING: usize = 1_000_000;

const MAX_PLUGIN_DIR_BYTES: usize = 4096;

const MAX_OBSERVABILITY_ADDR_BYTES: usize = 256;
const MAX_TRACING_ENDPOINT_BYTES: usize = 4096;

const MAX_SECRET_PREFIX_BYTES: usize = 128;
const MAX_SECRET_FILE_DIR_BYTES: usize = 4096;

const MAX_ONVIF_SCHEMES: usize = 16;
const MAX_ONVIF_SCHEME_BYTES: usize = 32;
const MAX_ONVIF_DEFAULT_TENANT_BYTES: usize = 128;
const MAX_ONVIF_DEFAULT_USERNAME_BYTES: usize = 256;
const MAX_ONVIF_CREDENTIALS_REF_BYTES: usize = 256;

const MAX_ONVIF_TIMEOUT_MS: i64 = 24 * 60 * 60 * 1_000; // 1 day
const MAX_ONVIF_INTERVAL_MS: i64 = 24 * 60 * 60 * 1_000; // 1 day
const MAX_ONVIF_TTL_MS: i64 = 30 * MAX_ONVIF_INTERVAL_MS; // 30 days

const MAX_ONVIF_RESPONSE_BYTES: usize = 64 * 1024 * 1024; // 64 MiB
const MAX_ONVIF_CONCURRENCY: usize = 1_000_000;
const MAX_ONVIF_ENDPOINT_CAPACITY: usize = 1_000_000;
const MAX_ONVIF_DATAGRAM_BYTES: usize = 2 * 1024 * 1024; // 2 MiB
const MAX_ONVIF_XML_DEPTH: usize = 10_000;
const MAX_ONVIF_XML_NODES: usize = 1_000_000;
const MAX_ONVIF_DISCOVERY_MATCHES: usize = 10_000;
const MAX_ONVIF_RATE_PER_SOURCE: u32 = 100_000;
const MAX_ONVIF_RATE_SOURCES: usize = 100_000;
const MAX_ONVIF_MAX_CONCURRENT_PROBES: u32 = 10_000;
const MAX_ONVIF_ALLOWED_PORTS: usize = 64;
const MAX_ONVIF_RATE_WINDOW_SECONDS: u64 = 24 * 60 * 60; // 1 day

const MAX_PLUGIN_INSTANCES: u32 = 10_000;

const MAX_CLUSTER_LEASE_TTL_MS: i64 = 24 * 60 * 60 * 1_000; // 1 day
const MAX_CLUSTER_HEARTBEAT_INTERVAL_MS: i64 = 24 * 60 * 60 * 1_000; // 1 day

const MAX_OBSERVABILITY_DIAGNOSTIC_DURATION_MS: u64 = 24 * 60 * 60 * 1_000; // 1 day
const MAX_OBSERVABILITY_DIAGNOSTIC_BODY_BYTES: usize = 64 * 1024 * 1024; // 64 MiB

const MAX_SECURITY_TOKEN_TTL_MS: i64 = 365 * 24 * 60 * 60 * 1_000; // 365 days
fn validate_string(field: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.len() > max_bytes {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} exceeds {max_bytes} bytes"),
        ));
    }
    Ok(())
}

fn validate_optional_string(field: &str, value: Option<&str>, max_bytes: usize) -> Result<()> {
    if let Some(value) = value {
        validate_string(field, value, max_bytes)?;
    }
    Ok(())
}

fn validate_secret(field: &str, value: &secrecy::SecretString, max_bytes: usize) -> Result<()> {
    let exposed = value.expose_secret();
    if exposed.len() > max_bytes {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} exceeds {max_bytes} bytes"),
        ));
    }
    Ok(())
}

fn validate_positive_i64(field: &str, value: i64, max: i64) -> Result<()> {
    if value <= 0 || value > max {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} must be between 1 and {max}"),
        ));
    }
    Ok(())
}

fn validate_nonnegative_i64(field: &str, value: i64, max: i64) -> Result<()> {
    if value < 0 || value > max {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} must be between 0 and {max}"),
        ));
    }
    Ok(())
}

fn validate_positive_u32(field: &str, value: u32, max: u32) -> Result<()> {
    if value == 0 || value > max {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} must be between 1 and {max}"),
        ));
    }
    Ok(())
}

fn validate_positive_u64(field: &str, value: u64, max: u64) -> Result<()> {
    if value == 0 || value > max {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} must be between 1 and {max}"),
        ));
    }
    Ok(())
}

fn validate_nonneg_usize(field: &str, value: usize, max: usize) -> Result<()> {
    if value > max {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} must be between 0 and {max}"),
        ));
    }
    Ok(())
}

fn validate_string_list(
    field: &str,
    values: &[String],
    max_count: usize,
    max_item_bytes: usize,
) -> Result<()> {
    if values.len() > max_count {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} must not exceed {max_count} entries"),
        ));
    }
    for (i, value) in values.iter().enumerate() {
        if value.is_empty() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("{field}[{i}] must not be empty"),
            ));
        }
        if value.len() > max_item_bytes {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("{field}[{i}] exceeds {max_item_bytes} bytes"),
            ));
        }
    }
    Ok(())
}

fn validate_positive_usize(field: &str, value: usize, max: usize) -> Result<()> {
    if value == 0 || value > max {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} must be between 1 and {max}"),
        ));
    }
    Ok(())
}

impl SystemConfig {
    /// Validates string field bounds for the system configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string(
            "system.node_name",
            &self.node_name,
            MAX_SYSTEM_NODE_NAME_BYTES,
        )?;
        validate_string("system.data_dir", &self.data_dir, MAX_SYSTEM_DATA_DIR_BYTES)?;
        validate_string(
            "system.log_level",
            self.log_level.trim(),
            MAX_SYSTEM_LOG_LEVEL_BYTES,
        )?;
        Ok(())
    }
}

impl SecurityConfig {
    /// Validates string/list/numeric field bounds for the security configuration.
    pub fn validate(&self) -> Result<()> {
        validate_secret(
            "security.jwt_public_key_ref",
            &self.jwt_public_key_ref,
            MAX_SECURITY_SECRET_REF_BYTES,
        )?;
        validate_string_list(
            "security.jwt_audience",
            &self.jwt_audience,
            MAX_JWT_ENTRIES,
            MAX_JWT_STRING_BYTES,
        )?;
        validate_string_list(
            "security.jwt_issuer",
            &self.jwt_issuer,
            MAX_JWT_ENTRIES,
            MAX_JWT_STRING_BYTES,
        )?;
        validate_secret(
            "security.api_key_hash",
            &self.api_key_hash,
            MAX_SECURITY_SECRET_REF_BYTES,
        )?;
        validate_secret(
            "security.static_api_key",
            &self.static_api_key,
            MAX_STATIC_API_KEY_BYTES,
        )?;
        validate_positive_i64(
            "security.token_ttl_ms",
            self.token_ttl_ms.as_millis(),
            MAX_SECURITY_TOKEN_TTL_MS,
        )?;
        Ok(())
    }
}

impl StorageConfig {
    /// Validates string and numeric field bounds for the storage configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string(
            "storage.sqlite_path",
            &self.sqlite_path,
            MAX_STORAGE_PATH_BYTES,
        )?;
        validate_secret(
            "storage.postgres_url",
            &self.postgres_url,
            MAX_POSTGRES_URL_BYTES,
        )?;
        validate_optional_string(
            "storage.postgres_url_ref",
            self.postgres_url_ref.as_deref(),
            MAX_STORAGE_SECRET_REF_BYTES,
        )?;
        validate_positive_u32(
            "storage.max_connections",
            self.max_connections,
            MAX_STORAGE_CONNECTIONS,
        )?;
        validate_positive_i64(
            "storage.connection_timeout_ms",
            self.connection_timeout_ms.as_millis(),
            MAX_STORAGE_CONNECTION_TIMEOUT_MS,
        )?;
        Ok(())
    }
}

impl MediaConfig {
    /// Validates string and numeric field bounds for the media configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string(
            "media.default_media_node_selector",
            &self.default_media_node_selector,
            MAX_MEDIA_NODE_SELECTOR_BYTES,
        )?;
        validate_positive_i64(
            "media.default_invite_timeout_ms",
            self.default_invite_timeout_ms.as_millis(),
            MAX_MEDIA_INVITE_TIMEOUT_MS,
        )?;
        validate_positive_i64(
            "media.periodic_reconcile_interval_ms",
            self.periodic_reconcile_interval_ms.as_millis(),
            MAX_MEDIA_RECONCILE_INTERVAL_MS,
        )?;
        validate_positive_i64(
            "media.needs_verification_grace_ms",
            self.needs_verification_grace_ms.as_millis(),
            MAX_MEDIA_VERIFICATION_GRACE_MS,
        )?;
        validate_positive_u32(
            "media.max_sessions_per_device",
            self.max_sessions_per_device,
            MAX_MEDIA_SESSIONS_PER_DEVICE,
        )?;
        Ok(())
    }
}

impl GrpcConfig {
    /// Validates string field bounds for the gRPC configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string("grpc.listen_addr", &self.listen_addr, MAX_GRPC_ADDR_BYTES)?;
        validate_optional_string(
            "grpc.tls_cert_ref",
            self.tls_cert_ref.as_deref(),
            MAX_GRPC_SECRET_REF_BYTES,
        )?;
        validate_optional_string(
            "grpc.tls_key_ref",
            self.tls_key_ref.as_deref(),
            MAX_GRPC_SECRET_REF_BYTES,
        )?;
        validate_optional_string(
            "grpc.mtls_client_ca_ref",
            self.mtls_client_ca_ref.as_deref(),
            MAX_GRPC_SECRET_REF_BYTES,
        )?;
        Ok(())
    }
}

impl MessagingConfig {
    /// Validates string field bounds and numeric limits for the messaging configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string(
            "messaging.nats_url",
            &self.nats_url,
            MAX_MESSAGING_URL_BYTES,
        )?;
        validate_optional_string(
            "messaging.nats_url_ref",
            self.nats_url_ref.as_deref(),
            MAX_MESSAGING_SECRET_REF_BYTES,
        )?;
        validate_string(
            "messaging.jetstream_domain",
            &self.jetstream_domain,
            MAX_MESSAGING_DOMAIN_BYTES,
        )?;
        if self.max_pending > MAX_MESSAGING_PENDING {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("messaging.max_pending must not exceed {MAX_MESSAGING_PENDING}"),
            ));
        }
        Ok(())
    }
}

impl PluginsConfig {
    /// Validates string and numeric field bounds for the plugins configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string("plugins.plugin_dir", &self.plugin_dir, MAX_PLUGIN_DIR_BYTES)?;
        validate_positive_u32(
            "plugins.max_plugin_instances",
            self.max_plugin_instances,
            MAX_PLUGIN_INSTANCES,
        )?;
        Ok(())
    }
}

impl ClusterConfig {
    /// Validates numeric field bounds for the cluster configuration.
    pub fn validate(&self) -> Result<()> {
        if self.enabled {
            validate_nonnegative_i64(
                "cluster.lease_ttl_ms",
                self.lease_ttl_ms.as_millis(),
                MAX_CLUSTER_LEASE_TTL_MS,
            )?;
            validate_nonnegative_i64(
                "cluster.heartbeat_interval_ms",
                self.heartbeat_interval_ms.as_millis(),
                MAX_CLUSTER_HEARTBEAT_INTERVAL_MS,
            )?;
        }
        Ok(())
    }
}

impl ObservabilityConfig {
    /// Validates string and numeric field bounds for the observability configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string(
            "observability.metrics_bind_addr",
            &self.metrics_bind_addr,
            MAX_OBSERVABILITY_ADDR_BYTES,
        )?;
        validate_optional_string(
            "observability.tracing_endpoint",
            self.tracing_endpoint.as_deref(),
            MAX_TRACING_ENDPOINT_BYTES,
        )?;
        if self.diagnostic_sample_rate < 0.0 || self.diagnostic_sample_rate > 1.0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "observability.diagnostic_sample_rate must be in [0.0, 1.0]",
            ));
        }
        if self.diagnostic_sample_rate > 0.0 {
            validate_positive_u64(
                "observability.diagnostic_max_duration_ms",
                self.diagnostic_max_duration_ms,
                MAX_OBSERVABILITY_DIAGNOSTIC_DURATION_MS,
            )?;
        }
        validate_nonneg_usize(
            "observability.diagnostic_max_body_bytes",
            self.diagnostic_max_body_bytes,
            MAX_OBSERVABILITY_DIAGNOSTIC_BODY_BYTES,
        )?;
        Ok(())
    }
}

impl SecretConfig {
    /// Validates string field bounds for the secret provider configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string(
            "secret.env_prefix",
            &self.env_prefix,
            MAX_SECRET_PREFIX_BYTES,
        )?;
        validate_optional_string(
            "secret.file_dir",
            self.file_dir.as_deref(),
            MAX_SECRET_FILE_DIR_BYTES,
        )?;
        Ok(())
    }
}

impl OnvifConfig {
    /// Validates string/list and numeric bounds for the ONVIF configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string_list(
            "onvif.allowed_schemes",
            &self.allowed_schemes,
            MAX_ONVIF_SCHEMES,
            MAX_ONVIF_SCHEME_BYTES,
        )?;

        if self.allowed_ports.len() > MAX_ONVIF_ALLOWED_PORTS {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("onvif.allowed_ports must not exceed {MAX_ONVIF_ALLOWED_PORTS} entries"),
            ));
        }
        for (i, port) in self.allowed_ports.iter().enumerate() {
            if *port == 0 {
                return Err(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    format!("onvif.allowed_ports[{i}] must not be zero"),
                ));
            }
        }

        validate_optional_string(
            "onvif.default_tenant_id",
            self.default_tenant_id.as_deref(),
            MAX_ONVIF_DEFAULT_TENANT_BYTES,
        )?;
        validate_optional_string(
            "onvif.default_username",
            self.default_username.as_deref(),
            MAX_ONVIF_DEFAULT_USERNAME_BYTES,
        )?;
        validate_optional_string(
            "onvif.default_credentials_ref",
            self.default_credentials_ref.as_deref(),
            MAX_ONVIF_CREDENTIALS_REF_BYTES,
        )?;

        validate_positive_i64(
            "onvif.connect_timeout_ms",
            self.connect_timeout_ms.as_millis(),
            MAX_ONVIF_TIMEOUT_MS,
        )?;
        validate_positive_i64(
            "onvif.request_timeout_ms",
            self.request_timeout_ms.as_millis(),
            MAX_ONVIF_TIMEOUT_MS,
        )?;
        validate_positive_i64(
            "onvif.probe_timeout_ms",
            self.probe_timeout_ms.as_millis(),
            MAX_ONVIF_TIMEOUT_MS,
        )?;
        validate_positive_i64(
            "onvif.discovery_timeout_ms",
            self.discovery_timeout_ms.as_millis(),
            MAX_ONVIF_TIMEOUT_MS,
        )?;
        validate_nonnegative_i64(
            "onvif.discovery_interval_ms",
            self.discovery_interval_ms.as_millis(),
            MAX_ONVIF_INTERVAL_MS,
        )?;
        validate_nonnegative_i64(
            "onvif.capability_ttl_ms",
            self.capability_ttl_ms.as_millis(),
            MAX_ONVIF_TTL_MS,
        )?;

        validate_positive_usize(
            "onvif.max_response_bytes",
            self.max_response_bytes,
            MAX_ONVIF_RESPONSE_BYTES,
        )?;
        validate_positive_usize(
            "onvif.max_concurrent_requests",
            self.max_concurrent_requests,
            MAX_ONVIF_CONCURRENCY,
        )?;
        validate_positive_usize(
            "onvif.per_device_concurrency",
            self.per_device_concurrency,
            MAX_ONVIF_CONCURRENCY,
        )?;
        validate_positive_usize(
            "onvif.max_tracked_device_endpoints",
            self.max_tracked_device_endpoints,
            MAX_ONVIF_ENDPOINT_CAPACITY,
        )?;
        validate_positive_usize(
            "onvif.discovery_max_datagram_bytes",
            self.discovery_max_datagram_bytes,
            MAX_ONVIF_DATAGRAM_BYTES,
        )?;
        validate_positive_usize(
            "onvif.discovery_max_xml_depth",
            self.discovery_max_xml_depth,
            MAX_ONVIF_XML_DEPTH,
        )?;
        validate_positive_usize(
            "onvif.discovery_max_xml_nodes",
            self.discovery_max_xml_nodes,
            MAX_ONVIF_XML_NODES,
        )?;
        validate_positive_usize(
            "onvif.discovery_max_matches",
            self.discovery_max_matches,
            MAX_ONVIF_DISCOVERY_MATCHES,
        )?;
        validate_positive_u32(
            "onvif.discovery_rate_max_per_source",
            self.discovery_rate_max_per_source,
            MAX_ONVIF_RATE_PER_SOURCE,
        )?;
        validate_positive_usize(
            "onvif.discovery_rate_max_sources",
            self.discovery_rate_max_sources,
            MAX_ONVIF_RATE_SOURCES,
        )?;
        validate_positive_u32(
            "onvif.max_concurrent_probes",
            self.max_concurrent_probes,
            MAX_ONVIF_MAX_CONCURRENT_PROBES,
        )?;
        validate_positive_usize(
            "onvif.capability_cache_capacity",
            self.capability_cache_capacity,
            MAX_ONVIF_ENDPOINT_CAPACITY,
        )?;
        validate_positive_u64(
            "onvif.discovery_rate_window_seconds",
            self.discovery_rate_window_seconds,
            MAX_ONVIF_RATE_WINDOW_SECONDS,
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DurationMs;

    #[test]
    fn system_config_rejects_long_node_name() {
        let config = SystemConfig {
            node_name: "x".repeat(MAX_SYSTEM_NODE_NAME_BYTES + 1),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn security_config_rejects_long_jwt_audience_entry() {
        let config = SecurityConfig {
            jwt_audience: vec!["x".repeat(MAX_JWT_STRING_BYTES + 1)],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn security_config_rejects_too_many_jwt_issuers() {
        let config = SecurityConfig {
            jwt_issuer: (0..=MAX_JWT_ENTRIES)
                .map(|i| format!("issuer-{i}"))
                .collect(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn storage_config_rejects_long_sqlite_path() {
        let config = StorageConfig {
            sqlite_path: "/".to_string() + &"x".repeat(MAX_STORAGE_PATH_BYTES),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn storage_config_rejects_zero_max_connections() {
        let config = StorageConfig {
            max_connections: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn storage_config_rejects_excessive_max_connections() {
        let config = StorageConfig {
            max_connections: MAX_STORAGE_CONNECTIONS + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn storage_config_rejects_zero_connection_timeout() {
        let config = StorageConfig {
            connection_timeout_ms: DurationMs::from_millis(0),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn storage_config_rejects_excessive_connection_timeout() {
        let config = StorageConfig {
            connection_timeout_ms: DurationMs::from_millis(MAX_STORAGE_CONNECTION_TIMEOUT_MS + 1),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn storage_config_limits_are_accepted() {
        let config = StorageConfig {
            max_connections: MAX_STORAGE_CONNECTIONS,
            connection_timeout_ms: DurationMs::from_millis(MAX_STORAGE_CONNECTION_TIMEOUT_MS),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn media_config_rejects_long_selector() {
        let config = MediaConfig {
            default_media_node_selector: "x".repeat(MAX_MEDIA_NODE_SELECTOR_BYTES + 1),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn media_config_rejects_zero_invite_timeout() {
        let config = MediaConfig {
            default_invite_timeout_ms: DurationMs::from_millis(0),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn media_config_rejects_excessive_reconcile_interval() {
        let config = MediaConfig {
            periodic_reconcile_interval_ms: DurationMs::from_millis(
                MAX_MEDIA_RECONCILE_INTERVAL_MS + 1,
            ),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn media_config_rejects_zero_max_sessions() {
        let config = MediaConfig {
            max_sessions_per_device: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn grpc_config_rejects_long_tls_ref() {
        let config = GrpcConfig {
            tls_cert_ref: Some("x".repeat(MAX_GRPC_SECRET_REF_BYTES + 1)),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn messaging_config_rejects_long_nats_url() {
        let config = MessagingConfig {
            nats_url: "nats://".to_string() + &"x".repeat(MAX_MESSAGING_URL_BYTES),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn messaging_config_rejects_excessive_max_pending() {
        let config = MessagingConfig {
            max_pending: MAX_MESSAGING_PENDING + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn messaging_config_accepts_zero_max_pending() {
        let config = MessagingConfig {
            max_pending: 0,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn observability_config_rejects_long_tracing_endpoint() {
        let config = ObservabilityConfig {
            tracing_endpoint: Some("http://".to_string() + &"x".repeat(MAX_TRACING_ENDPOINT_BYTES)),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn observability_config_rejects_zero_diagnostic_duration_when_sampling() {
        let config = ObservabilityConfig {
            diagnostic_sample_rate: 1.0,
            diagnostic_max_duration_ms: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn observability_config_rejects_excessive_diagnostic_body_bytes() {
        let config = ObservabilityConfig {
            diagnostic_max_body_bytes: MAX_OBSERVABILITY_DIAGNOSTIC_BODY_BYTES + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn plugins_config_rejects_zero_max_instances() {
        let config = PluginsConfig {
            max_plugin_instances: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn cluster_config_rejects_excessive_lease_ttl_when_enabled() {
        let config = ClusterConfig {
            enabled: true,
            lease_ttl_ms: crate::DurationMs::from_millis(MAX_CLUSTER_LEASE_TTL_MS + 1),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn security_config_rejects_zero_token_ttl() {
        let config = SecurityConfig {
            token_ttl_ms: crate::DurationMs::from_millis(0),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn security_config_rejects_excessive_token_ttl() {
        let config = SecurityConfig {
            token_ttl_ms: crate::DurationMs::from_millis(MAX_SECURITY_TOKEN_TTL_MS + 1),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_rejects_empty_allowed_scheme() {
        let config = OnvifConfig {
            allowed_schemes: vec!["http".to_string(), "".to_string()],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_accepts_defaults() {
        assert!(OnvifConfig::default().validate().is_ok());
    }

    #[test]
    fn onvif_config_rejects_negative_timeout() {
        let config = OnvifConfig {
            connect_timeout_ms: DurationMs::from_millis(-1),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_rejects_excessive_timeout() {
        let config = OnvifConfig {
            request_timeout_ms: DurationMs::from_millis(MAX_ONVIF_TIMEOUT_MS + 1),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_rejects_zero_response_bytes() {
        let config = OnvifConfig {
            max_response_bytes: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_rejects_excessive_response_bytes() {
        let config = OnvifConfig {
            max_response_bytes: MAX_ONVIF_RESPONSE_BYTES + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_rejects_zero_port_in_allowed_ports() {
        let config = OnvifConfig {
            allowed_ports: vec![0],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_rejects_too_many_allowed_ports() {
        let config = OnvifConfig {
            allowed_ports: (1..=MAX_ONVIF_ALLOWED_PORTS + 1)
                .map(|p| p as u16)
                .collect(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_rejects_negative_discovery_interval() {
        let config = OnvifConfig {
            discovery_interval_ms: DurationMs::from_millis(-1),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn onvif_config_accepts_zero_discovery_interval_and_ttl() {
        let config = OnvifConfig {
            discovery_interval_ms: DurationMs::from_millis(0),
            capability_ttl_ms: DurationMs::from_millis(0),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn onvif_config_rejects_excessive_max_concurrent_probes() {
        let config = OnvifConfig {
            max_concurrent_probes: MAX_ONVIF_MAX_CONCURRENT_PROBES + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}

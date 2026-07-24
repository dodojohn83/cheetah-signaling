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

fn validate_positive_i64(field: &str, value: i64, max: i64) -> Result<()> {
    if value <= 0 || value > max {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            format!("{field} must be between 1 and {max}"),
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
    /// Validates string/list field bounds for the security configuration.
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
        Ok(())
    }
}

impl StorageConfig {
    /// Validates string field bounds for the storage configuration.
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
    /// Validates string field bounds for the plugins configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string("plugins.plugin_dir", &self.plugin_dir, MAX_PLUGIN_DIR_BYTES)?;
        Ok(())
    }
}

impl ObservabilityConfig {
    /// Validates string field bounds for the observability configuration.
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
    /// Validates string/list field bounds for the ONVIF configuration.
    pub fn validate(&self) -> Result<()> {
        validate_string_list(
            "onvif.allowed_schemes",
            &self.allowed_schemes,
            MAX_ONVIF_SCHEMES,
            MAX_ONVIF_SCHEME_BYTES,
        )?;
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
    fn onvif_config_rejects_empty_allowed_scheme() {
        let config = OnvifConfig {
            allowed_schemes: vec!["http".to_string(), "".to_string()],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}

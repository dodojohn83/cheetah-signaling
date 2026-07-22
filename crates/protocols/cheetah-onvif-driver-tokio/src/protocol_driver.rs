//! Real `ProtocolDriver` adapter for the Tokio ONVIF HTTP/SOAP implementation.
//!
//! This module maps the plugin SDK's generic `ProtocolDriver` commands to the
//! lower-level `OnvifHttpDriver` methods and exposes a factory that can be
//! registered with the `PluginHost`.

use crate::{DeviceCredentials, DriverConfig, DriverError, OnvifHttpDriver};
use async_trait::async_trait;
use cheetah_onvif_module::services::MediaDialect;
use cheetah_plugin_sdk::{
    CapabilityDescriptor, DriverCommand, DriverContext, HealthReport, HealthStatus, PluginError,
    PluginName, ProtocolCapability, ProtocolDirection, ProtocolDriver, ProtocolDriverFactory,
};
use cheetah_signal_types::DurationMs;
use cheetah_signal_types::config::OnvifConfig;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, warn};

const PROTOCOL: &str = "onvif";

/// Tokio-backed ONVIF protocol driver.
#[derive(Debug, Clone, Copy, Default)]
pub struct OnvifTokioProtocolDriver;

impl OnvifTokioProtocolDriver {
    /// Creates a new driver instance.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProtocolDriver for OnvifTokioProtocolDriver {
    async fn start(
        &self,
        ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        // Validate that the supplied configuration can build a driver.
        let config = onvif_config(ctx);
        let _driver = build_driver(&config)?;
        debug!("onvif tokio driver started");
        Ok(())
    }

    async fn drain(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    async fn shutdown(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    async fn handle_command(
        &self,
        ctx: &dyn DriverContext,
        command: DriverCommand,
        timeout: DurationMs,
    ) -> Result<(), PluginError> {
        let config = onvif_config(ctx);
        let driver = build_driver(&config)?;
        let timeout = effective_timeout(timeout, &driver);
        dispatch_command(ctx, &config, &driver, &command, timeout).await
    }

    async fn probe(
        &self,
        ctx: &dyn DriverContext,
        target: &str,
        timeout: DurationMs,
    ) -> Result<CapabilityDescriptor, PluginError> {
        let config = onvif_config(ctx);
        let driver = build_driver(&config)?;
        let timeout = effective_timeout(timeout, &driver);
        driver
            .get_system_date_and_time(target, timeout)
            .await
            .map_err(plugin_error_from_driver_error)?;
        Ok(CapabilityDescriptor {
            protocol: PROTOCOL.to_string(),
            direction: ProtocolDirection::Outbound,
            metadata: HashMap::new(),
        })
    }

    async fn health(
        &self,
        ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<HealthReport, PluginError> {
        let config = onvif_config(ctx);
        match build_driver(&config) {
            Ok(_) => Ok(HealthReport {
                status: HealthStatus::Healthy,
                message: "ONVIF Tokio driver config valid".to_string(),
                metrics: HashMap::new(),
            }),
            Err(e) => Ok(HealthReport {
                status: HealthStatus::Unhealthy,
                message: e.to_string(),
                metrics: HashMap::new(),
            }),
        }
    }
}

/// Factory for [`OnvifTokioProtocolDriver`].
#[derive(Debug, Clone, Copy, Default)]
pub struct OnvifTokioDriverFactory;

impl OnvifTokioDriverFactory {
    /// Creates a new factory.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProtocolDriverFactory for OnvifTokioDriverFactory {
    fn name(&self) -> PluginName {
        PluginName::from_static_unchecked("cheetah/onvif")
    }

    fn capabilities(&self) -> Vec<ProtocolCapability> {
        vec![ProtocolCapability {
            protocol: PROTOCOL.to_string(),
            direction: ProtocolDirection::Outbound,
            media_transport: None,
        }]
    }

    async fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn ProtocolDriver>, PluginError> {
        Ok(Box::new(OnvifTokioProtocolDriver::new()))
    }
}

fn build_driver(config: &OnvifConfig) -> Result<OnvifHttpDriver, PluginError> {
    let driver_config = DriverConfig::from(config);
    OnvifHttpDriver::new(&driver_config).map_err(plugin_error_from_driver_error)
}

fn onvif_config(ctx: &dyn DriverContext) -> OnvifConfig {
    if ctx.config().is_null() {
        OnvifConfig::default()
    } else {
        serde_json::from_value::<OnvifConfig>(ctx.config().clone()).unwrap_or_else(|e| {
            warn!(error = %e, "failed to parse onvif config; using defaults");
            OnvifConfig::default()
        })
    }
}

fn effective_timeout(timeout: DurationMs, driver: &OnvifHttpDriver) -> Option<Duration> {
    let _ = driver;
    if timeout.as_millis() > 0 {
        Some(Duration::from_millis(timeout.as_millis() as u64))
    } else {
        None
    }
}

async fn dispatch_command(
    ctx: &dyn DriverContext,
    config: &OnvifConfig,
    driver: &OnvifHttpDriver,
    command: &DriverCommand,
    timeout: Option<Duration>,
) -> Result<(), PluginError> {
    match command.command_type.as_str() {
        "get_device_information" => {
            let cmd: EndpointCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .get_device_information(&cmd.endpoint, credentials.as_ref(), timeout)
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "get_system_date_and_time" => {
            let cmd: EndpointCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            driver
                .get_system_date_and_time(&cmd.endpoint, timeout)
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "get_profiles" => {
            let cmd: MediaCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .get_profiles(
                    &cmd.media_endpoint,
                    cmd.dialect(),
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "get_stream_uri" => {
            let cmd: StreamUriCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .get_stream_uri(
                    &cmd.media_endpoint,
                    cmd.dialect(),
                    &cmd.profile_token,
                    &cmd.protocol,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "get_snapshot_uri" => {
            let cmd: SnapshotUriCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .get_snapshot_uri(
                    &cmd.media_endpoint,
                    cmd.dialect(),
                    &cmd.profile_token,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        _ => {
            return Err(PluginError::Unsupported(format!(
                "onvif command {} is not supported by the tokio driver",
                command.command_type
            )));
        }
    }
    Ok(())
}

fn parse_payload<T: for<'de> Deserialize<'de>>(
    payload: &serde_json::Value,
) -> Result<T, PluginError> {
    serde_json::from_value(payload.clone())
        .map_err(|e| PluginError::Driver(format!("invalid onvif command payload: {e}")))
}

fn plugin_error_from_driver_error(e: DriverError) -> PluginError {
    match e {
        DriverError::Onvif(e) => PluginError::Driver(e.to_string()),
        DriverError::Module(e) => PluginError::Driver(e.to_string()),
        DriverError::Http(s) => PluginError::Transient(s),
        DriverError::HttpStatus { status, body } => {
            PluginError::Driver(format!("http status {status}: {body}"))
        }
        DriverError::BodyLimit { limit } => {
            PluginError::Driver(format!("response body limit exceeded: {limit} bytes"))
        }
        DriverError::Io(e) => PluginError::Transient(e.to_string()),
        DriverError::Timeout(_) => PluginError::Cancelled,
        DriverError::Config(s) => PluginError::Driver(s),
    }
}

#[derive(Deserialize)]
struct EndpointCommand {
    endpoint: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    credentials_ref: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    password_text: bool,
    #[serde(default)]
    clock_offset_seconds: i64,
}

impl EndpointCommand {
    async fn resolve_credentials(
        &self,
        ctx: &dyn DriverContext,
        config: &OnvifConfig,
    ) -> Result<Option<DeviceCredentials>, PluginError> {
        resolve_credentials(
            ctx,
            config,
            self.username.as_deref(),
            self.credentials_ref.as_deref(),
            self.password.as_deref(),
            self.password_text,
            self.clock_offset_seconds,
        )
        .await
    }

    fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
struct MediaCommand {
    media_endpoint: String,
    #[serde(default)]
    dialect: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    credentials_ref: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    password_text: bool,
    #[serde(default)]
    clock_offset_seconds: i64,
}

impl MediaCommand {
    fn dialect(&self) -> MediaDialect {
        parse_dialect(self.dialect.as_deref())
    }

    async fn resolve_credentials(
        &self,
        ctx: &dyn DriverContext,
        config: &OnvifConfig,
    ) -> Result<Option<DeviceCredentials>, PluginError> {
        resolve_credentials(
            ctx,
            config,
            self.username.as_deref(),
            self.credentials_ref.as_deref(),
            self.password.as_deref(),
            self.password_text,
            self.clock_offset_seconds,
        )
        .await
    }

    fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
struct StreamUriCommand {
    media_endpoint: String,
    #[serde(default)]
    dialect: Option<String>,
    profile_token: String,
    #[serde(default = "default_stream_protocol")]
    protocol: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    credentials_ref: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    password_text: bool,
    #[serde(default)]
    clock_offset_seconds: i64,
}

impl StreamUriCommand {
    fn dialect(&self) -> MediaDialect {
        parse_dialect(self.dialect.as_deref())
    }

    async fn resolve_credentials(
        &self,
        ctx: &dyn DriverContext,
        config: &OnvifConfig,
    ) -> Result<Option<DeviceCredentials>, PluginError> {
        resolve_credentials(
            ctx,
            config,
            self.username.as_deref(),
            self.credentials_ref.as_deref(),
            self.password.as_deref(),
            self.password_text,
            self.clock_offset_seconds,
        )
        .await
    }

    fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
struct SnapshotUriCommand {
    media_endpoint: String,
    #[serde(default)]
    dialect: Option<String>,
    profile_token: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    credentials_ref: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    password_text: bool,
    #[serde(default)]
    clock_offset_seconds: i64,
}

impl SnapshotUriCommand {
    fn dialect(&self) -> MediaDialect {
        parse_dialect(self.dialect.as_deref())
    }

    async fn resolve_credentials(
        &self,
        ctx: &dyn DriverContext,
        config: &OnvifConfig,
    ) -> Result<Option<DeviceCredentials>, PluginError> {
        resolve_credentials(
            ctx,
            config,
            self.username.as_deref(),
            self.credentials_ref.as_deref(),
            self.password.as_deref(),
            self.password_text,
            self.clock_offset_seconds,
        )
        .await
    }

    fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

fn default_stream_protocol() -> String {
    "UDP".to_string()
}

fn parse_dialect(dialect: Option<&str>) -> MediaDialect {
    match dialect {
        Some("media1") => MediaDialect::Media1,
        Some("media2") => MediaDialect::Media2,
        _ => MediaDialect::Media2,
    }
}

async fn resolve_credentials(
    ctx: &dyn DriverContext,
    config: &OnvifConfig,
    username: Option<&str>,
    credentials_ref: Option<&str>,
    password: Option<&str>,
    password_text: bool,
    clock_offset_seconds: i64,
) -> Result<Option<DeviceCredentials>, PluginError> {
    let effective_username = username.or(config.default_username.as_deref());

    let password_secret: Option<SecretString> = if let Some(ref_name) = credentials_ref {
        match ctx.secret(ref_name).await? {
            Some(secret) => Some(secret),
            None => {
                return Err(PluginError::Driver(format!(
                    "credentials secret {ref_name} not found"
                )));
            }
        }
    } else if password.is_some() {
        password.map(SecretString::from)
    } else if let Some(ref_name) = config.default_credentials_ref.as_deref() {
        match ctx.secret(ref_name).await? {
            Some(secret) => Some(secret),
            None => {
                return Err(PluginError::Driver(format!(
                    "credentials secret {ref_name} not found"
                )));
            }
        }
    } else {
        None
    };

    if effective_username.is_none() && password_secret.is_none() {
        return Ok(None);
    }

    if let (Some(u), None) = (effective_username, password_secret.as_ref())
        && !u.is_empty()
    {
        return Err(PluginError::Driver(
            "username provided without a password or credentials_ref".into(),
        ));
    }

    Ok(make_credentials(
        effective_username,
        password_secret,
        password_text,
        clock_offset_seconds,
    ))
}

fn make_credentials(
    username: Option<&str>,
    password: Option<SecretString>,
    password_text: bool,
    clock_offset_seconds: i64,
) -> Option<DeviceCredentials> {
    let username = username?.to_string();
    let password = password?;
    if username.is_empty() || password.expose_secret().is_empty() {
        return None;
    }
    Some(DeviceCredentials {
        username,
        password,
        password_text,
        clock_offset_seconds,
    })
}

fn command_timeout(timeout_ms: Option<u64>, default: Option<Duration>) -> Option<Duration> {
    timeout_ms
        .filter(|&ms| ms > 0)
        .map(Duration::from_millis)
        .or(default)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default
    )]

    use super::*;
    use cheetah_plugin_sdk::{CommandSource, DeviceSink, ProtocolEvent, ResourceBudget};

    #[test]
    fn parse_dialect_values() {
        assert_eq!(parse_dialect(Some("media1")), MediaDialect::Media1);
        assert_eq!(parse_dialect(Some("media2")), MediaDialect::Media2);
        assert_eq!(parse_dialect(None), MediaDialect::Media2);
        assert_eq!(parse_dialect(Some("unknown")), MediaDialect::Media2);
    }

    #[test]
    fn credentials_require_both_username_and_password() {
        use secrecy::SecretString;

        assert!(
            make_credentials(
                Some("admin"),
                Some(SecretString::from("secret".to_string())),
                false,
                0
            )
            .is_some()
        );
        assert!(make_credentials(Some("admin"), None, false, 0).is_none());
        assert!(
            make_credentials(
                None,
                Some(SecretString::from("secret".to_string())),
                false,
                0
            )
            .is_none()
        );
        assert!(
            make_credentials(
                Some(""),
                Some(SecretString::from("secret".to_string())),
                false,
                0
            )
            .is_none()
        );
        assert!(
            make_credentials(
                Some("admin"),
                Some(SecretString::from("".to_string())),
                false,
                0
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn resolve_credentials_prefers_secret_provider_ref() {
        let ctx = FakeDriverContext::with_secrets(&[
            ("onvif.default.password", "default_secret"),
            ("per_device_password", "device_secret"),
        ]);
        let mut config = OnvifConfig::default();
        config.default_username = Some("admin".to_string());
        config.default_credentials_ref = Some("onvif.default.password".to_string());

        let creds = resolve_credentials(
            &ctx,
            &config,
            Some("device_user"),
            Some("per_device_password"),
            None,
            false,
            0,
        )
        .await
        .expect("resolve should succeed")
        .expect("credentials should be present");

        assert_eq!(creds.username, "device_user");
        assert_eq!(creds.password.expose_secret(), "device_secret");
    }

    #[tokio::test]
    async fn resolve_credentials_falls_back_to_config_defaults() {
        let ctx = FakeDriverContext::with_secret("onvif.default.password", "fallback");
        let mut config = OnvifConfig::default();
        config.default_username = Some("admin".to_string());
        config.default_credentials_ref = Some("onvif.default.password".to_string());

        let creds = resolve_credentials(&ctx, &config, None, None, None, false, 0)
            .await
            .expect("resolve should succeed")
            .expect("credentials should be present");

        assert_eq!(creds.username, "admin");
        assert_eq!(creds.password.expose_secret(), "fallback");
    }

    #[tokio::test]
    async fn resolve_credentials_returns_error_for_missing_secret() {
        let ctx = FakeDriverContext::with_secret("other", "value");
        let mut config = OnvifConfig::default();
        config.default_credentials_ref = Some("missing".to_string());

        let err = resolve_credentials(&ctx, &config, Some("admin"), None, None, false, 0)
            .await
            .expect_err("missing secret should error");
        assert!(err.to_string().contains("missing"));
    }

    #[tokio::test]
    async fn resolve_credentials_prefers_inline_password_over_config_default() {
        let ctx = FakeDriverContext::with_secret("onvif.default.password", "default_secret");
        let mut config = OnvifConfig::default();
        config.default_username = Some("admin".to_string());
        config.default_credentials_ref = Some("onvif.default.password".to_string());

        let creds = resolve_credentials(
            &ctx,
            &config,
            Some("device_user"),
            None,
            Some("inline_secret"),
            false,
            0,
        )
        .await
        .expect("resolve should succeed")
        .expect("credentials should be present");

        assert_eq!(creds.username, "device_user");
        assert_eq!(creds.password.expose_secret(), "inline_secret");
    }

    struct FakeDeviceSink;

    #[async_trait]
    impl DeviceSink for FakeDeviceSink {
        async fn emit_event(&self, _event: ProtocolEvent) -> Result<(), PluginError> {
            Ok(())
        }
    }

    struct FakeCommandSource;

    #[async_trait]
    impl CommandSource for FakeCommandSource {
        async fn next_command(&self) -> Result<Option<DriverCommand>, PluginError> {
            Ok(None)
        }
    }

    struct FakeDriverContext {
        secrets: HashMap<String, SecretString>,
    }

    impl FakeDriverContext {
        fn with_secret(name: &str, value: &str) -> Self {
            let mut secrets = HashMap::new();
            secrets.insert(name.to_string(), SecretString::from(value.to_string()));
            Self { secrets }
        }

        fn with_secrets(pairs: &[(&str, &str)]) -> Self {
            let secrets = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), SecretString::from(v.to_string())))
                .collect();
            Self { secrets }
        }
    }

    #[async_trait]
    impl DriverContext for FakeDriverContext {
        fn plugin_name(&self) -> &PluginName {
            use std::sync::LazyLock;
            static NAME: LazyLock<PluginName> =
                LazyLock::new(|| PluginName::new("cheetah/test").unwrap());
            &NAME
        }

        fn config(&self) -> &serde_json::Value {
            use std::sync::LazyLock;
            static CONFIG: LazyLock<serde_json::Value> = LazyLock::new(|| serde_json::Value::Null);
            &CONFIG
        }

        fn budget(&self) -> &ResourceBudget {
            use std::sync::LazyLock;
            static BUDGET: LazyLock<ResourceBudget> = LazyLock::new(ResourceBudget::default);
            &BUDGET
        }

        fn monotonic_now(&self) -> cheetah_plugin_sdk::MonotonicSeconds {
            0
        }

        fn device_sink(&self) -> &dyn DeviceSink {
            static SINK: FakeDeviceSink = FakeDeviceSink;
            &SINK
        }

        fn command_source(&self) -> &dyn CommandSource {
            static SOURCE: FakeCommandSource = FakeCommandSource;
            &SOURCE
        }

        async fn secret(&self, name: &str) -> Result<Option<SecretString>, PluginError> {
            Ok(self.secrets.get(name).cloned())
        }

        async fn request_media_session(
            &self,
            _params: serde_json::Value,
            _timeout: DurationMs,
        ) -> Result<String, PluginError> {
            unimplemented!()
        }

        async fn register_endpoint(
            &self,
            _protocol: &str,
            _address: &str,
        ) -> Result<String, PluginError> {
            unimplemented!()
        }
    }
}

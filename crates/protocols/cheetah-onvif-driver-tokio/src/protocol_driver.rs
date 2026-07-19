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
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, warn};

const PROTOCOL: &str = "onvif";

/// Tokio-backed ONVIF protocol driver.
pub struct OnvifTokioProtocolDriver {
    // Shared driver instance built lazily from the supplied plugin context.
    // Reusing the same `OnvifHttpDriver` preserves the underlying reqwest
    // connection pool and enforces the configured concurrency limit.
    driver: Mutex<Option<OnvifHttpDriver>>,
}

impl fmt::Debug for OnvifTokioProtocolDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnvifTokioProtocolDriver").finish_non_exhaustive()
    }
}

impl OnvifTokioProtocolDriver {
    /// Creates a new driver instance.
    pub fn new() -> Self {
        Self {
            driver: Mutex::new(None),
        }
    }

    fn driver(&self, ctx: &dyn DriverContext) -> Result<OnvifHttpDriver, PluginError> {
        let mut guard = self
            .driver
            .lock()
            .map_err(|e| PluginError::Driver(format!("onvif driver lock poisoned: {e}")))?;
        if let Some(driver) = guard.as_ref() {
            return Ok(driver.clone());
        }
        let driver = build_driver(ctx)?;
        let _ = guard.insert(driver.clone());
        Ok(driver)
    }
}

impl Default for OnvifTokioProtocolDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProtocolDriver for OnvifTokioProtocolDriver {
    async fn start(
        &self,
        ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        // Validate that the supplied configuration can build a driver and cache it.
        let _driver = self.driver(ctx)?;
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
        let driver = self.driver(ctx)?;
        let timeout = effective_timeout(timeout, &driver);
        dispatch_command(&driver, &command, timeout).await
    }

    async fn probe(
        &self,
        ctx: &dyn DriverContext,
        target: &str,
        timeout: DurationMs,
    ) -> Result<CapabilityDescriptor, PluginError> {
        let driver = self.driver(ctx)?;
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
        match self.driver(ctx) {
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
        match PluginName::new("cheetah/onvif") {
            Ok(name) => name,
            Err(e) => panic!("static plugin name 'cheetah/onvif' is invalid: {e}"),
        }
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

fn build_driver(ctx: &dyn DriverContext) -> Result<OnvifHttpDriver, PluginError> {
    let config = onvif_config(ctx);
    let driver_config = DriverConfig::from(&config);
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
    driver: &OnvifHttpDriver,
    command: &DriverCommand,
    timeout: Option<Duration>,
) -> Result<(), PluginError> {
    match command.command_type.as_str() {
        "get_device_information" => {
            let cmd: EndpointCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            driver
                .get_device_information(&cmd.endpoint, cmd.credentials().as_ref(), timeout)
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
            driver
                .get_profiles(
                    &cmd.media_endpoint,
                    cmd.dialect(),
                    cmd.credentials().as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "get_stream_uri" => {
            let cmd: StreamUriCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            driver
                .get_stream_uri(
                    &cmd.media_endpoint,
                    cmd.dialect(),
                    &cmd.profile_token,
                    &cmd.protocol,
                    cmd.credentials().as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "get_snapshot_uri" => {
            let cmd: SnapshotUriCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            driver
                .get_snapshot_uri(
                    &cmd.media_endpoint,
                    cmd.dialect(),
                    &cmd.profile_token,
                    cmd.credentials().as_ref(),
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
    password: Option<String>,
    #[serde(default)]
    password_text: bool,
    #[serde(default)]
    clock_offset_seconds: i64,
}

impl EndpointCommand {
    fn credentials(&self) -> Option<DeviceCredentials> {
        make_credentials(
            self.username.as_deref(),
            self.password.as_deref(),
            self.password_text,
            self.clock_offset_seconds,
        )
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

    fn credentials(&self) -> Option<DeviceCredentials> {
        make_credentials(
            self.username.as_deref(),
            self.password.as_deref(),
            self.password_text,
            self.clock_offset_seconds,
        )
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

    fn credentials(&self) -> Option<DeviceCredentials> {
        make_credentials(
            self.username.as_deref(),
            self.password.as_deref(),
            self.password_text,
            self.clock_offset_seconds,
        )
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

    fn credentials(&self) -> Option<DeviceCredentials> {
        make_credentials(
            self.username.as_deref(),
            self.password.as_deref(),
            self.password_text,
            self.clock_offset_seconds,
        )
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

fn make_credentials(
    username: Option<&str>,
    password: Option<&str>,
    password_text: bool,
    clock_offset_seconds: i64,
) -> Option<DeviceCredentials> {
    let username = username?.to_string();
    let password = password?;
    if username.is_empty() || password.is_empty() {
        return None;
    }
    Some(DeviceCredentials {
        username,
        password: SecretString::from(password.to_string()),
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
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn parse_dialect_values() {
        assert_eq!(parse_dialect(Some("media1")), MediaDialect::Media1);
        assert_eq!(parse_dialect(Some("media2")), MediaDialect::Media2);
        assert_eq!(parse_dialect(None), MediaDialect::Media2);
        assert_eq!(parse_dialect(Some("unknown")), MediaDialect::Media2);
    }

    #[test]
    fn credentials_require_both_username_and_password() {
        assert!(make_credentials(Some("admin"), Some("secret"), false, 0).is_some());
        assert!(make_credentials(Some("admin"), None, false, 0).is_none());
        assert!(make_credentials(None, Some("secret"), false, 0).is_none());
        assert!(make_credentials(Some(""), Some("secret"), false, 0).is_none());
        assert!(make_credentials(Some("admin"), Some(""), false, 0).is_none());
    }
}

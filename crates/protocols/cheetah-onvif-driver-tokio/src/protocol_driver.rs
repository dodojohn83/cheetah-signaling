//! Real `ProtocolDriver` adapter for the Tokio ONVIF HTTP/SOAP implementation.
//!
//! This module maps the plugin SDK's generic `ProtocolDriver` commands to the
//! lower-level `OnvifHttpDriver` methods and exposes a factory that can be
//! registered with the `PluginHost`.

use crate::{DeviceCredentials, DriverConfig, DriverError, OnvifHttpDriver};
use async_trait::async_trait;
use cheetah_onvif_module::services::{
    MediaDialect, OnvifNotification, PtzPreset, PtzVelocity, PullPointSubscription, SnapshotUri,
    SystemDateAndTime, clip_unit, normalize_topic, redact_uri_userinfo,
};
use cheetah_onvif_module::{CapabilityKind, CapabilityProbeResult, Service};
use cheetah_plugin_sdk::{
    CapabilityDescriptor, DriverCommand, DriverContext, HealthReport, HealthStatus, PluginError,
    PluginName, ProtocolCapability, ProtocolDirection, ProtocolDriver, ProtocolDriverFactory,
    ProtocolEvent,
};
use cheetah_signal_types::DurationMs;
use cheetah_signal_types::TenantId;
use cheetah_signal_types::UtcTimestamp;
use cheetah_signal_types::config::OnvifConfig;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, warn};

const PROTOCOL: &str = "onvif";

/// Tokio-backed ONVIF protocol driver.
///
/// The driver is stateful: the `OnvifHttpDriver` (SOAP client + limits) is
/// created once during `start` and reused across commands. This avoids
/// re-parsing configuration and re-creating the HTTP client on every request.
#[derive(Debug, Clone, Default)]
pub struct OnvifTokioProtocolDriver {
    driver: Arc<Mutex<Option<OnvifHttpDriver>>>,
}

impl OnvifTokioProtocolDriver {
    /// Creates a new driver instance.
    pub fn new() -> Self {
        Self::default()
    }

    fn get_or_build_driver(&self, ctx: &dyn DriverContext) -> Result<OnvifHttpDriver, PluginError> {
        {
            let guard = self
                .driver
                .lock()
                .map_err(|e| PluginError::Driver(format!("driver mutex poisoned: {e:?}")))?;
            if let Some(driver) = guard.as_ref() {
                return Ok(driver.clone());
            }
        }
        let config = onvif_config(ctx);
        let driver = build_driver(&config)?;
        {
            let mut guard = self
                .driver
                .lock()
                .map_err(|e| PluginError::Driver(format!("driver mutex poisoned: {e:?}")))?;
            *guard = Some(driver.clone());
        }
        Ok(driver)
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
        let _driver = self.get_or_build_driver(ctx)?;
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
        let driver = self.get_or_build_driver(ctx)?;
        let config = onvif_config(ctx);
        let timeout = effective_timeout(timeout, &driver);
        dispatch_command(ctx, &config, &driver, &command, timeout).await
    }

    async fn probe(
        &self,
        ctx: &dyn DriverContext,
        target: &str,
        timeout: DurationMs,
    ) -> Result<CapabilityDescriptor, PluginError> {
        let driver = self.get_or_build_driver(ctx)?;
        let config = onvif_config(ctx);
        let timeout = effective_timeout(timeout, &driver);
        let system_date_and_time = driver
            .get_system_date_and_time(target, timeout)
            .await
            .map_err(plugin_error_from_driver_error)?;

        let mut metadata = HashMap::new();
        metadata.insert("onvif_endpoint".to_string(), target.to_string());

        // Persist the device clock offset and the wall time at which the probe ran.
        let offset_seconds = clock_offset_seconds(&system_date_and_time)?;
        metadata.insert(
            "onvif_clock_offset_seconds".to_string(),
            offset_seconds.to_string(),
        );

        if let Some(credentials_ref) = &config.default_credentials_ref {
            metadata.insert(
                "onvif_default_credentials_ref".to_string(),
                credentials_ref.clone(),
            );
        }
        if let Some(username) = &config.default_username {
            metadata.insert("onvif_default_username".to_string(), username.clone());
        }

        // Use configured default credentials, if any, to probe services and capabilities.
        // Failures here are recorded in metadata but do not fail the probe so that
        // reachable devices without credentials still report basic availability.
        let fetched_at = UtcTimestamp::from_offset(time::OffsetDateTime::now_utc())
            .to_rfc3339()
            .unwrap_or_default();
        match resolve_credentials(ctx, &config, None, None, None, false, 0).await {
            Ok(Some(credentials)) => {
                match driver
                    .get_services(target, None, false, Some(&credentials), timeout)
                    .await
                {
                    Ok(services) => {
                        metadata.insert("services".to_string(), services_to_json(&services));
                        metadata
                            .insert("onvif_services_fetched_at".to_string(), fetched_at.clone());
                    }
                    Err(e) => {
                        metadata.insert("services_error".to_string(), e.to_string());
                    }
                }
                match driver
                    .get_capabilities(target, None, Some(&credentials), timeout)
                    .await
                {
                    Ok(caps) => {
                        metadata.insert("capabilities".to_string(), capabilities_to_json(&caps));
                        metadata.insert("onvif_capabilities_fetched_at".to_string(), fetched_at);
                    }
                    Err(e) => {
                        metadata.insert("capabilities_error".to_string(), e.to_string());
                    }
                }
            }
            Ok(None) => {
                metadata.insert(
                    "credentials_error".to_string(),
                    "no credentials available".to_string(),
                );
            }
            Err(e) => {
                metadata.insert("credentials_error".to_string(), e.to_string());
            }
        }

        Ok(CapabilityDescriptor {
            protocol: PROTOCOL.to_string(),
            direction: ProtocolDirection::Outbound,
            metadata,
        })
    }

    async fn health(
        &self,
        ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<HealthReport, PluginError> {
        let config = onvif_config(ctx);
        let driver_result = self.get_or_build_driver(ctx);

        let driver_ready = driver_result.is_ok();
        let queue_saturated = matches!(&driver_result, Ok(d) if d.is_request_queue_saturated());
        let mut credentials_available = true;
        if let Some(ref_name) = config.default_credentials_ref.as_deref() {
            credentials_available = matches!(ctx.secret(ref_name).await, Ok(Some(_)));
        }
        let dependency_degraded =
            driver_result.is_err() || !credentials_available || queue_saturated;

        let status = if !driver_ready {
            HealthStatus::Unhealthy
        } else if queue_saturated || !credentials_available {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        let message = match &driver_result {
            Ok(_) => "ONVIF Tokio driver ready".to_string(),
            Err(e) => format!("ONVIF Tokio driver not ready: {e}"),
        };

        let mut metrics = HashMap::new();
        metrics.insert("driver_ready".to_string(), if driver_ready { 1 } else { 0 });
        metrics.insert(
            "credentials_available".to_string(),
            if credentials_available { 1 } else { 0 },
        );
        metrics.insert(
            "queue_saturated".to_string(),
            if queue_saturated { 1 } else { 0 },
        );
        metrics.insert(
            "dependency_degraded".to_string(),
            if dependency_degraded { 1 } else { 0 },
        );

        Ok(HealthReport {
            status,
            message,
            metrics,
        })
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
        "get_services" => {
            let cmd: EndpointCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .get_services(
                    &cmd.endpoint,
                    cmd.tenant_id.as_deref(),
                    cmd.include_capabilities,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "get_capabilities" => {
            let cmd: EndpointCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .get_capabilities(
                    &cmd.endpoint,
                    cmd.tenant_id.as_deref(),
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "take_snapshot" => {
            let cmd: SnapshotUriCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let uri = driver
                .get_snapshot_uri(
                    &cmd.media_endpoint,
                    cmd.dialect(),
                    &cmd.profile_token,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
            emit_snapshot(ctx, tenant_id, &uri).await?;
        }
        "ptz_get_presets" => {
            let cmd: PtzGetPresetsCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let presets = driver
                .get_ptz_presets(
                    &cmd.ptz_endpoint,
                    &cmd.profile_token,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
            emit_ptz_presets(ctx, tenant_id, &cmd.profile_token, &presets).await?;
        }
        "ptz_continuous_move" => {
            let cmd: PtzContinuousMoveCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .ptz_continuous_move(
                    &cmd.ptz_endpoint,
                    &cmd.profile_token,
                    cmd.velocity(),
                    cmd.timeout_seconds,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "ptz_stop" => {
            let cmd: PtzStopCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .ptz_stop(
                    &cmd.ptz_endpoint,
                    &cmd.profile_token,
                    cmd.pan_tilt,
                    cmd.zoom,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "create_pull_point_subscription" => {
            let cmd: CreatePullPointSubscriptionCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let sub = driver
                .create_pull_point_subscription(
                    &cmd.events_endpoint,
                    &cmd.initial_termination_time,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
            emit_pull_point_subscription(ctx, tenant_id, &sub).await?;
        }
        "pull_messages" => {
            let cmd: PullMessagesCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let messages = driver
                .pull_messages(
                    &cmd.subscription_reference,
                    &cmd.timeout,
                    cmd.message_limit,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
            if !messages.is_empty() {
                emit_onvif_notifications(ctx, tenant_id, messages).await?;
            }
        }
        "renew_pull_point_subscription" => {
            let cmd: RenewPullPointSubscriptionCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .renew_pull_point_subscription(
                    &cmd.subscription_reference,
                    &cmd.termination_time,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        "unsubscribe_pull_point" => {
            let cmd: UnsubscribePullPointCommand = parse_payload(&command.payload)?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            driver
                .unsubscribe_pull_point(&cmd.subscription_reference, credentials.as_ref(), timeout)
                .await
                .map_err(plugin_error_from_driver_error)?;
        }
        // Imaging v1 write commands are explicitly rejected; they are not part of
        // the signaling control plane and would require media-node coordination.
        "set_imaging_settings"
        | "set_focus_configuration"
        | "set_exposure"
        | "set_white_balance"
        | "set_backlight_compensation"
        | "set_wide_dynamic_range"
        | "set_defog"
        | "set_iris_filter"
        | "set_focus" => {
            return Err(PluginError::Unsupported(format!(
                "onvif imaging write command {} is not supported",
                command.command_type
            )));
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
    tenant_id: Option<String>,
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
    #[serde(default)]
    include_capabilities: bool,
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
    #[serde(default)]
    tenant_id: Option<String>,
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

#[derive(Deserialize)]
struct PtzGetPresetsCommand {
    ptz_endpoint: String,
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
    #[serde(default)]
    tenant_id: Option<String>,
}

impl PtzGetPresetsCommand {
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
struct PtzContinuousMoveCommand {
    ptz_endpoint: String,
    profile_token: String,
    pan: f64,
    tilt: f64,
    zoom: f64,
    timeout_seconds: u64,
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

impl PtzContinuousMoveCommand {
    fn velocity(&self) -> PtzVelocity {
        PtzVelocity {
            pan: clip_unit(self.pan),
            tilt: clip_unit(self.tilt),
            zoom: clip_unit(self.zoom),
        }
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
struct PtzStopCommand {
    ptz_endpoint: String,
    profile_token: String,
    #[serde(default = "default_true")]
    pan_tilt: bool,
    #[serde(default = "default_true")]
    zoom: bool,
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

impl PtzStopCommand {
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
struct CreatePullPointSubscriptionCommand {
    events_endpoint: String,
    initial_termination_time: String,
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
    #[serde(default)]
    tenant_id: Option<String>,
}

impl CreatePullPointSubscriptionCommand {
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
struct PullMessagesCommand {
    subscription_reference: String,
    timeout: String,
    message_limit: u32,
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
    #[serde(default)]
    tenant_id: Option<String>,
}

impl PullMessagesCommand {
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
struct RenewPullPointSubscriptionCommand {
    subscription_reference: String,
    termination_time: String,
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

impl RenewPullPointSubscriptionCommand {
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
struct UnsubscribePullPointCommand {
    subscription_reference: String,
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

impl UnsubscribePullPointCommand {
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

fn default_true() -> bool {
    true
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

fn parse_tenant_id(raw: Option<&str>) -> Result<Option<TenantId>, PluginError> {
    match raw {
        None | Some("") => Ok(None),
        Some(s) => s
            .parse()
            .map(Some)
            .map_err(|e| PluginError::Driver(format!("invalid tenant_id {s}: {e}"))),
    }
}

fn snapshot_event_payload(uri: &SnapshotUri) -> serde_json::Value {
    serde_json::json!({
        "uri": redact_uri_userinfo(&uri.uri),
        "invalid_after_connect": uri.invalid_after_connect,
        "invalid_after_reboot": uri.invalid_after_reboot,
        "timeout": uri.timeout,
    })
}

async fn emit_snapshot(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    uri: &SnapshotUri,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.snapshot_uri".into(),
            payload: snapshot_event_payload(uri),
            tenant_id,
        })
        .await
}

fn ptz_presets_event_payload(profile_token: &str, presets: &[PtzPreset]) -> serde_json::Value {
    serde_json::json!({
        "profile_token": profile_token,
        "presets": presets.iter().map(|p| serde_json::json!({"token": p.token, "name": p.name})).collect::<Vec<_>>(),
    })
}

async fn emit_ptz_presets(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    profile_token: &str,
    presets: &[PtzPreset],
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.ptz_presets".into(),
            payload: ptz_presets_event_payload(profile_token, presets),
            tenant_id,
        })
        .await
}

fn notifications_event_payload(notifications: &[OnvifNotification]) -> serde_json::Value {
    serde_json::json!({
        "notifications": notifications
            .iter()
            .map(|n| serde_json::json!({
                "topic": normalize_topic(&n.topic),
                "utc_time": n.utc_time,
                "property_operation": n.property_operation,
            }))
            .collect::<Vec<_>>(),
    })
}

async fn emit_onvif_notifications(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    messages: Vec<OnvifNotification>,
) -> Result<(), PluginError> {
    if messages.is_empty() {
        return Ok(());
    }
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.notification".into(),
            payload: notifications_event_payload(&messages),
            tenant_id,
        })
        .await
}

fn pull_point_subscription_event_payload(sub: &PullPointSubscription) -> serde_json::Value {
    serde_json::json!({
        "subscription_reference": redact_uri_userinfo(&sub.subscription_reference),
        "termination_time": sub.termination_time,
        "current_time": sub.current_time,
    })
}

async fn emit_pull_point_subscription(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    sub: &PullPointSubscription,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.pull_point_subscription".into(),
            payload: pull_point_subscription_event_payload(sub),
            tenant_id,
        })
        .await
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

fn services_to_json(services: &[Service]) -> String {
    let values: Vec<serde_json::Value> = services
        .iter()
        .map(|s| {
            serde_json::json!({
                "namespace": s.namespace,
                "xaddr": s.xaddr,
                "version": s.version,
            })
        })
        .collect();
    serde_json::to_string(&values).unwrap_or_default()
}

fn capabilities_to_json(
    caps: &std::collections::HashMap<CapabilityKind, CapabilityProbeResult>,
) -> String {
    let mut map = serde_json::Map::new();
    for (kind, result) in caps {
        let value = match result {
            CapabilityProbeResult::Supported {
                namespace,
                xaddr,
                version,
            } => serde_json::json!({
                "status": "supported",
                "namespace": namespace,
                "xaddr": xaddr,
                "version": version,
            }),
            CapabilityProbeResult::Unsupported => serde_json::json!({"status": "unsupported"}),
            CapabilityProbeResult::Failed { reason, retryable } => serde_json::json!({
                "status": "failed",
                "reason": reason,
                "retryable": retryable,
            }),
        };
        map.insert(kind.to_string(), value);
    }
    serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_default()
}

fn clock_offset_seconds(system: &SystemDateAndTime) -> Result<i64, PluginError> {
    clock_offset_seconds_with_local(system, time::OffsetDateTime::now_utc())
}

fn clock_offset_seconds_with_local(
    system: &SystemDateAndTime,
    local_utc: time::OffsetDateTime,
) -> Result<i64, PluginError> {
    let device_utc = system
        .utc
        .to_utc()
        .map_err(|e| PluginError::Driver(e.to_string()))?;
    Ok((device_utc - local_utc).whole_seconds())
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
            Err(PluginError::Unsupported(
                "media session not available in tests".into(),
            ))
        }

        async fn register_endpoint(
            &self,
            _protocol: &str,
            _address: &str,
        ) -> Result<String, PluginError> {
            Err(PluginError::Unsupported(
                "endpoint registration not available in tests".into(),
            ))
        }
    }

    #[tokio::test]
    async fn imaging_write_commands_return_unsupported() {
        use cheetah_signal_types::UtcTimestamp;

        let driver = OnvifTokioProtocolDriver::new();
        let ctx = FakeDriverContext::with_secret("onvif.default.password", "secret");
        let deadline = UtcTimestamp::parse_rfc3339("9999-12-31T23:59:59Z").unwrap();

        for command_type in [
            "set_imaging_settings",
            "set_focus_configuration",
            "set_exposure",
            "set_white_balance",
            "set_focus",
        ] {
            let command = DriverCommand {
                command_type: command_type.to_string(),
                payload: serde_json::json!({}),
                idempotency_key: format!("test-{command_type}"),
                deadline,
            };
            let result = driver
                .handle_command(&ctx, command, DurationMs::from_millis(1_000))
                .await;
            assert!(
                matches!(result, Err(PluginError::Unsupported(_))),
                "{command_type} should be unsupported, got {result:?}"
            );
        }
    }

    #[test]
    fn clock_offset_matches_device_minus_local_time() {
        use cheetah_onvif_core::services::system_date_time::{DateTime, SystemDateAndTime};

        let local = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let device = local + time::Duration::seconds(37);
        let system = SystemDateAndTime {
            date_time_type: "NTP".to_string(),
            daylight_savings: false,
            timezone: None,
            utc: DateTime {
                year: device.year(),
                month: device.month() as u8,
                day: device.day(),
                hour: device.hour(),
                minute: device.minute(),
                second: device.second(),
            },
            local: None,
        };

        let offset = clock_offset_seconds_with_local(&system, local).unwrap();
        assert_eq!(offset, 37);
    }

    #[test]
    fn ptz_continuous_move_clips_velocity_components_to_unit_range() {
        let cmd = PtzContinuousMoveCommand {
            ptz_endpoint: "http://192.0.2.10/onvif/ptz".into(),
            profile_token: "profile1".into(),
            pan: 1.5,
            tilt: -2.0,
            zoom: 0.5,
            timeout_seconds: 5,
            timeout_ms: None,
            username: None,
            credentials_ref: None,
            password: None,
            password_text: false,
            clock_offset_seconds: 0,
        };
        let velocity = cmd.velocity();
        assert_eq!(velocity.pan, 1.0);
        assert_eq!(velocity.tilt, -1.0);
        assert_eq!(velocity.zoom, 0.5);
    }

    #[test]
    fn snapshot_event_payload_redacts_uri_userinfo() {
        let uri = SnapshotUri {
            uri: "http://user:pass@192.0.2.10/snapshot".into(),
            invalid_after_connect: Some(false),
            invalid_after_reboot: Some(false),
            timeout: None,
        };
        let payload = snapshot_event_payload(&uri);
        let emitted = payload["uri"].as_str().unwrap();
        assert!(!emitted.contains("pass"));
        assert!(emitted.contains("192.0.2.10"));
        assert_eq!(payload["invalid_after_connect"].as_bool(), Some(false));
    }

    #[test]
    fn ptz_presets_event_payload_uses_command_profile_token() {
        let presets = vec![PtzPreset {
            token: "preset-token".into(),
            name: "Home".into(),
        }];
        let payload = ptz_presets_event_payload("profile-token", &presets);
        assert_eq!(payload["profile_token"].as_str().unwrap(), "profile-token");
        assert_eq!(payload["presets"].as_array().unwrap().len(), presets.len());
    }

    #[test]
    fn parse_tenant_id_rejects_malformed_input() {
        assert!(parse_tenant_id(None).unwrap().is_none());
        assert!(parse_tenant_id(Some("")).unwrap().is_none());
        assert!(parse_tenant_id(Some("not-a-uuid")).is_err());
    }
}

//! Real `ProtocolDriver` adapter for the Tokio ONVIF HTTP/SOAP implementation.
//!
//! This module maps the plugin SDK's generic `ProtocolDriver` commands to the
//! lower-level `OnvifHttpDriver` methods and exposes a factory that can be
//! registered with the `PluginHost`.

use crate::commands::*;
use crate::util::clamp_timeout;
use crate::{DriverConfig, DriverError, OnvifHttpDriver, events};
use async_trait::async_trait;
use cheetah_onvif_services::services::{SystemDateAndTime, redact_uri_userinfo};
use cheetah_plugin_sdk::{
    CapabilityDescriptor, DriverCommand, DriverContext, HealthReport, HealthStatus, PluginError,
    PluginName, ProtocolCapability, ProtocolDirection, ProtocolDriver, ProtocolDriverFactory,
};
use cheetah_signal_types::config::OnvifConfig;
use cheetah_signal_types::{DurationMs, UtcTimestamp, clamp_str};

/// Maximum byte length of a metadata value stored in the probe
/// [`CapabilityDescriptor`]. This matches the bound enforced by
/// [`CapabilityDescriptor::validate`].
const MAX_PROBE_METADATA_VALUE_BYTES: usize = 4096;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::debug;
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
        let mut guard = self
            .driver
            .lock()
            .map_err(|e| PluginError::driver(format!("driver mutex poisoned: {e:?}")))?;
        if let Some(driver) = guard.as_ref() {
            return Ok(driver.clone());
        }
        let config = onvif_config(ctx)?;
        let driver = build_driver(&config)?;
        *guard = Some(driver.clone());
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
        let config = onvif_config(ctx)?;
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
        let config = onvif_config(ctx)?;
        let timeout = effective_timeout(timeout, &driver);
        let system_date_and_time = driver
            .get_system_date_and_time(target, timeout)
            .await
            .map_err(plugin_error_from_driver_error)?;

        let mut metadata = HashMap::new();
        metadata.insert(
            "onvif_endpoint".to_string(),
            clamp_str(&redact_uri_userinfo(target), MAX_PROBE_METADATA_VALUE_BYTES),
        );

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
                        metadata.insert("services_count".to_string(), services.len().to_string());
                        metadata
                            .insert("onvif_services_fetched_at".to_string(), fetched_at.clone());
                    }
                    Err(e) => {
                        metadata.insert(
                            "services_error".to_string(),
                            clamp_str(&e.to_string(), MAX_PROBE_METADATA_VALUE_BYTES),
                        );
                    }
                }
                match driver
                    .get_capabilities(target, None, Some(&credentials), timeout)
                    .await
                {
                    Ok(caps) => {
                        metadata.insert("capabilities_count".to_string(), caps.len().to_string());
                        metadata.insert("onvif_capabilities_fetched_at".to_string(), fetched_at);
                    }
                    Err(e) => {
                        metadata.insert(
                            "capabilities_error".to_string(),
                            clamp_str(&e.to_string(), MAX_PROBE_METADATA_VALUE_BYTES),
                        );
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
                metadata.insert(
                    "credentials_error".to_string(),
                    clamp_str(&e.to_string(), MAX_PROBE_METADATA_VALUE_BYTES),
                );
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
        let config = match onvif_config(ctx) {
            Ok(config) => config,
            Err(e) => {
                let mut metrics = HashMap::new();
                metrics.insert("driver_ready".to_string(), 0);
                metrics.insert("credentials_available".to_string(), 0);
                metrics.insert("queue_saturated".to_string(), 0);
                metrics.insert("dependency_degraded".to_string(), 1);
                return Ok(HealthReport {
                    status: HealthStatus::Unhealthy,
                    message: format!("invalid onvif config: {e}"),
                    metrics,
                });
            }
        };
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

fn onvif_config(ctx: &dyn DriverContext) -> Result<OnvifConfig, PluginError> {
    if ctx.config().is_null() {
        Ok(OnvifConfig::default())
    } else {
        serde_json::from_value::<OnvifConfig>(ctx.config().clone())
            .map_err(|e| PluginError::driver(format!("invalid onvif config: {e}")))
    }
}

fn effective_timeout(timeout: DurationMs, _driver: &OnvifHttpDriver) -> Option<Duration> {
    if timeout.as_millis() > 0 {
        Some(clamp_timeout(Duration::from_millis(
            timeout.as_millis() as u64
        )))
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
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let info = driver
                .get_device_information(&cmd.endpoint, credentials.as_ref(), timeout)
                .await
                .map_err(plugin_error_from_driver_error)?;
            events::emit_device_information(ctx, tenant_id, &info, &command.idempotency_key)
                .await?;
        }
        "get_system_date_and_time" => {
            let cmd: EndpointCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let timeout = cmd.command_timeout(timeout);
            let dt = driver
                .get_system_date_and_time(&cmd.endpoint, timeout)
                .await
                .map_err(plugin_error_from_driver_error)?;
            events::emit_system_date_and_time(ctx, tenant_id, &dt, &command.idempotency_key)
                .await?;
        }
        "get_profiles" => {
            let cmd: MediaCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let (dialect, profiles) = driver
                .get_profiles(
                    &cmd.media_endpoint,
                    cmd.dialect(),
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
            events::emit_profiles(ctx, tenant_id, dialect, &profiles, &command.idempotency_key)
                .await?;
        }
        "get_stream_uri" => {
            let cmd: StreamUriCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let uri = driver
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
            events::emit_stream_uri(
                ctx,
                tenant_id,
                &cmd.profile_token,
                &uri,
                &command.idempotency_key,
            )
            .await?;
        }
        "get_snapshot_uri" => {
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
            events::emit_snapshot(
                ctx,
                tenant_id,
                &cmd.profile_token,
                &uri,
                &command.idempotency_key,
            )
            .await?;
        }
        "get_services" => {
            let cmd: EndpointCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let tenant_str = tenant_id.as_ref().map(|t| t.to_string());
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let services = driver
                .get_services(
                    &cmd.endpoint,
                    tenant_str.as_deref(),
                    cmd.include_capabilities,
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
            events::emit_services(ctx, tenant_id, &services, &command.idempotency_key).await?;
        }
        "get_capabilities" => {
            let cmd: EndpointCommand = parse_payload(&command.payload)?;
            let tenant_id = parse_tenant_id(cmd.tenant_id.as_deref())?;
            let tenant_str = tenant_id.as_ref().map(|t| t.to_string());
            let timeout = cmd.command_timeout(timeout);
            let credentials = cmd.resolve_credentials(ctx, config).await?;
            let caps = driver
                .get_capabilities(
                    &cmd.endpoint,
                    tenant_str.as_deref(),
                    credentials.as_ref(),
                    timeout,
                )
                .await
                .map_err(plugin_error_from_driver_error)?;
            events::emit_capabilities(ctx, tenant_id, &caps, &command.idempotency_key).await?;
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
            events::emit_snapshot(
                ctx,
                tenant_id,
                &cmd.profile_token,
                &uri,
                &command.idempotency_key,
            )
            .await?;
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
            events::emit_ptz_presets(
                ctx,
                tenant_id,
                &cmd.profile_token,
                &presets,
                &command.idempotency_key,
            )
            .await?;
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
            events::emit_pull_point_subscription(ctx, tenant_id, &sub, &command.idempotency_key)
                .await?;
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
            events::emit_onvif_notifications(ctx, tenant_id, messages, &command.idempotency_key)
                .await?;
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
            return Err(PluginError::unsupported(format!(
                "onvif imaging write command {} is not supported",
                command.command_type
            )));
        }
        _ => {
            return Err(PluginError::unsupported(format!(
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
        .map_err(|e| PluginError::driver(format!("invalid onvif command payload: {e}")))
}

fn plugin_error_from_driver_error(e: DriverError) -> PluginError {
    match e {
        DriverError::Onvif(e) => PluginError::driver(e),
        DriverError::Module(e) => PluginError::driver(e),
        DriverError::Http(s) => PluginError::transient(s),
        DriverError::HttpStatus { status, body } => {
            PluginError::driver(format!("http status {status}: {body}"))
        }
        DriverError::BodyLimit { limit } => {
            PluginError::driver(format!("response body limit exceeded: {limit} bytes"))
        }
        DriverError::Io(e) => PluginError::transient(e),
        DriverError::Timeout(_) => PluginError::Cancelled,
        DriverError::Config(s) => PluginError::driver(s),
        DriverError::Overloaded(s) => PluginError::transient(s),
    }
}

fn clock_offset_seconds(system: &SystemDateAndTime) -> Result<i64, PluginError> {
    clock_offset_seconds_with_local(system, time::OffsetDateTime::now_utc())
}

fn clock_offset_seconds_with_local(
    system: &SystemDateAndTime,
    local_utc: time::OffsetDateTime,
) -> Result<i64, PluginError> {
    let device_utc = system.utc.to_utc().map_err(PluginError::driver)?;
    Ok((device_utc - local_utc).whole_seconds())
}

#[cfg(test)]
#[path = "protocol_driver_tests.rs"]
mod tests;

//! Typed ONVIF command payloads and credential resolution helpers.

use crate::DeviceCredentials;
use crate::util::clamp_timeout;
use cheetah_onvif_services::services::{MediaDialect, PtzVelocity, clip_unit};
use cheetah_plugin_sdk::{DriverContext, PluginError};
use cheetah_signal_types::{TenantId, config::OnvifConfig};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
pub(crate) struct EndpointCommand {
    pub(crate) endpoint: String,
    #[serde(default)]
    pub(crate) tenant_id: Option<String>,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
    #[serde(default)]
    pub(crate) include_capabilities: bool,
}

impl EndpointCommand {
    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct MediaCommand {
    pub(crate) media_endpoint: String,
    #[serde(default)]
    pub(crate) dialect: Option<String>,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
    #[serde(default)]
    pub(crate) tenant_id: Option<String>,
}

impl MediaCommand {
    pub(crate) fn dialect(&self) -> MediaDialect {
        parse_dialect(self.dialect.as_deref())
    }

    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct StreamUriCommand {
    pub(crate) media_endpoint: String,
    #[serde(default)]
    pub(crate) dialect: Option<String>,
    pub(crate) profile_token: String,
    #[serde(default = "default_stream_protocol")]
    pub(crate) protocol: String,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
    #[serde(default)]
    pub(crate) tenant_id: Option<String>,
}

impl StreamUriCommand {
    pub(crate) fn dialect(&self) -> MediaDialect {
        parse_dialect(self.dialect.as_deref())
    }

    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct SnapshotUriCommand {
    pub(crate) media_endpoint: String,
    #[serde(default)]
    pub(crate) dialect: Option<String>,
    pub(crate) profile_token: String,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
    #[serde(default)]
    pub(crate) tenant_id: Option<String>,
}

impl SnapshotUriCommand {
    pub(crate) fn dialect(&self) -> MediaDialect {
        parse_dialect(self.dialect.as_deref())
    }

    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct PtzGetPresetsCommand {
    pub(crate) ptz_endpoint: String,
    pub(crate) profile_token: String,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
    #[serde(default)]
    pub(crate) tenant_id: Option<String>,
}

impl PtzGetPresetsCommand {
    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct PtzContinuousMoveCommand {
    pub(crate) ptz_endpoint: String,
    pub(crate) profile_token: String,
    pub(crate) pan: f64,
    pub(crate) tilt: f64,
    pub(crate) zoom: f64,
    pub(crate) timeout_seconds: u64,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
}

impl PtzContinuousMoveCommand {
    pub(crate) fn velocity(&self) -> PtzVelocity {
        PtzVelocity {
            pan: clip_unit(self.pan),
            tilt: clip_unit(self.tilt),
            zoom: clip_unit(self.zoom),
        }
    }

    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct PtzStopCommand {
    pub(crate) ptz_endpoint: String,
    pub(crate) profile_token: String,
    #[serde(default = "default_true")]
    pub(crate) pan_tilt: bool,
    #[serde(default = "default_true")]
    pub(crate) zoom: bool,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
}

impl PtzStopCommand {
    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct CreatePullPointSubscriptionCommand {
    pub(crate) events_endpoint: String,
    pub(crate) initial_termination_time: String,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
    #[serde(default)]
    pub(crate) tenant_id: Option<String>,
}

impl CreatePullPointSubscriptionCommand {
    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct PullMessagesCommand {
    pub(crate) subscription_reference: String,
    pub(crate) timeout: String,
    pub(crate) message_limit: u32,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
    #[serde(default)]
    pub(crate) tenant_id: Option<String>,
}

impl PullMessagesCommand {
    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct RenewPullPointSubscriptionCommand {
    pub(crate) subscription_reference: String,
    pub(crate) termination_time: String,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
}

impl RenewPullPointSubscriptionCommand {
    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

#[derive(Deserialize)]
pub(crate) struct UnsubscribePullPointCommand {
    pub(crate) subscription_reference: String,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) credentials_ref: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) password_text: bool,
    #[serde(default)]
    pub(crate) clock_offset_seconds: i64,
}

impl UnsubscribePullPointCommand {
    pub(crate) async fn resolve_credentials(
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

    pub(crate) fn command_timeout(&self, default: Option<Duration>) -> Option<Duration> {
        command_timeout(self.timeout_ms, default)
    }
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_stream_protocol() -> String {
    "UDP".to_string()
}

pub(crate) fn parse_dialect(dialect: Option<&str>) -> MediaDialect {
    match dialect {
        Some(s) if s.eq_ignore_ascii_case("media1") => MediaDialect::Media1,
        Some(s) if s.eq_ignore_ascii_case("media2") => MediaDialect::Media2,
        _ => MediaDialect::Media2,
    }
}

pub(crate) fn parse_tenant_id(raw: Option<&str>) -> Result<Option<TenantId>, PluginError> {
    match raw {
        None | Some("") => Ok(None),
        Some(s) => s
            .parse()
            .map(Some)
            .map_err(|e| PluginError::driver(format!("invalid tenant_id {s}: {e}"))),
    }
}

pub(crate) async fn resolve_credentials(
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
                return Err(PluginError::driver(format!(
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
                return Err(PluginError::driver(format!(
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
        return Err(PluginError::driver(
            "username provided without a password or credentials_ref",
        ));
    }

    make_credentials(
        effective_username,
        password_secret,
        password_text,
        clock_offset_seconds,
    )
}

/// Maximum byte length for an ONVIF username.
pub(crate) const MAX_ONVIF_USERNAME_BYTES: usize = 256;
/// Maximum byte length for an ONVIF password.
pub(crate) const MAX_ONVIF_PASSWORD_BYTES: usize = 4096;

pub(crate) fn make_credentials(
    username: Option<&str>,
    password: Option<SecretString>,
    password_text: bool,
    clock_offset_seconds: i64,
) -> Result<Option<DeviceCredentials>, PluginError> {
    let Some(username) = username else {
        return Ok(None);
    };
    let Some(password) = password else {
        return Ok(None);
    };
    if username.is_empty() || password.expose_secret().is_empty() {
        return Ok(None);
    }
    if username.len() > MAX_ONVIF_USERNAME_BYTES {
        return Err(PluginError::driver("ONVIF username exceeds maximum length"));
    }
    if password.expose_secret().len() > MAX_ONVIF_PASSWORD_BYTES {
        return Err(PluginError::driver("ONVIF password exceeds maximum length"));
    }
    Ok(Some(DeviceCredentials {
        username: username.to_string(),
        password,
        password_text,
        clock_offset_seconds,
    }))
}

pub(crate) fn command_timeout(
    timeout_ms: Option<u64>,
    default: Option<Duration>,
) -> Option<Duration> {
    timeout_ms
        .filter(|&ms| ms > 0)
        .map(Duration::from_millis)
        .or(default)
        .map(clamp_timeout)
}

//! Protocol driver port adapter for the GB28181 access module.
//!
//! This is a minimal built-in driver that exposes the Sans-I/O access state
//! machine through the shared [`cheetah_plugin_sdk::ProtocolDriver`] trait.
//! Commands are delivered as JSON; outputs are emitted back to the host as
//! [`ProtocolEvent`]s.

use crate::{
    AccessInput, AccessOutput, Gb28181Access,
    config::{AuthPolicy, Gb28181DomainConfig},
    events::Gb28181Event,
    ports::CredentialProvider,
    types::DeviceId,
};
use async_trait::async_trait;
use cheetah_gb28181_core::{SipParser, SipParserConfig, sip::encoder::encode_message};
use cheetah_plugin_sdk::{
    CapabilityDescriptor, DriverCommand, DriverContext, HealthReport, HealthStatus, PluginError,
    PluginName, ProtocolCapability, ProtocolDirection, ProtocolDriver, ProtocolDriverFactory,
    ProtocolEvent,
};
use cheetah_signal_types::DurationMs;
use secrecy::{ExposeSecret, SecretSlice, SecretString};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::Mutex;

/// Credential provider that never returns a password.
///
/// The driver configures the access state machine with `ChallengeOptional`
/// authentication so unauthenticated devices can complete the digest challenge
/// flow. Per-device passwords are supplied by the host through the context
/// when command dispatch is extended later.
#[derive(Debug)]
struct NoopCredentialProvider;

impl CredentialProvider for NoopCredentialProvider {
    fn password_for(&self, _device_id: &DeviceId) -> Option<SecretString> {
        None
    }
}

/// Built-in GB28181 protocol driver.
pub struct Gb28181ProtocolDriver {
    inner: Mutex<Option<Gb28181Access<NoopCredentialProvider>>>,
}

impl std::fmt::Debug for Gb28181ProtocolDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181ProtocolDriver")
            .field("started", &true)
            .finish()
    }
}

impl Gb28181ProtocolDriver {
    /// Creates a driver. `start` must be called before processing commands.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

impl Default for Gb28181ProtocolDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProtocolDriver for Gb28181ProtocolDriver {
    async fn start(
        &self,
        ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        let config = load_domain_config(ctx).await?;
        let access = Gb28181Access::new(config, NoopCredentialProvider)
            .map_err(|e| PluginError::Driver(e.to_string()))?;
        let mut guard = self.inner.lock().await;
        *guard = Some(access);
        Ok(())
    }

    async fn drain(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        // Access state is in-memory; drain is a no-op beyond stopping new work.
        Ok(())
    }

    async fn shutdown(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        let mut guard = self.inner.lock().await;
        *guard = None;
        Ok(())
    }

    async fn handle_command(
        &self,
        ctx: &dyn DriverContext,
        command: DriverCommand,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        match command.command_type.as_str() {
            "process_sip" => process_sip(self, ctx, &command.payload).await,
            other => Err(PluginError::Unsupported(format!(
                "command {other} is not supported by the built-in GB28181 driver"
            ))),
        }
    }

    async fn probe(
        &self,
        _ctx: &dyn DriverContext,
        target: &str,
        _timeout: DurationMs,
    ) -> Result<CapabilityDescriptor, PluginError> {
        let _ = target
            .parse::<SocketAddr>()
            .map_err(|e| PluginError::Driver(format!("invalid target address: {e}")))?;
        Ok(CapabilityDescriptor {
            protocol: "gb28181".to_string(),
            direction: ProtocolDirection::Bidirectional,
            metadata: HashMap::new(),
        })
    }

    async fn health(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<HealthReport, PluginError> {
        let guard = self.inner.lock().await;
        let active = guard.is_some();
        let mut metrics = HashMap::new();
        metrics.insert("active".to_string(), u64::from(active));
        Ok(HealthReport {
            status: if active {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy
            },
            message: if active {
                "GB28181 access driver is active".to_string()
            } else {
                "GB28181 access driver has not been started".to_string()
            },
            metrics,
        })
    }
}

/// Factory for creating [`Gb28181ProtocolDriver`] instances.
#[derive(Debug)]
pub struct Gb28181DriverFactory;

impl Gb28181DriverFactory {
    /// Creates a new factory.
    pub fn new() -> Self {
        Self
    }
}

impl Default for Gb28181DriverFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProtocolDriverFactory for Gb28181DriverFactory {
    #[allow(clippy::expect_used)]
    fn name(&self) -> PluginName {
        PluginName::new("cheetah/gb28181").expect("valid built-in plugin name")
    }

    fn capabilities(&self) -> Vec<ProtocolCapability> {
        vec![ProtocolCapability {
            protocol: "gb28181".to_string(),
            direction: ProtocolDirection::Bidirectional,
            media_transport: Some("RTP/AVP".to_string()),
        }]
    }

    async fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn ProtocolDriver>, PluginError> {
        Ok(Box::new(Gb28181ProtocolDriver::new()))
    }
}

/// Parses the driver configuration JSON into a [`Gb28181DomainConfig`].
///
/// The digest secret is read from the host secret provider using the
/// `digest_secret_name` reference; it is never stored in the driver config.
async fn load_domain_config(ctx: &dyn DriverContext) -> Result<Gb28181DomainConfig, PluginError> {
    let cfg: DriverConfig = serde_json::from_value(ctx.config().clone())
        .map_err(|e| PluginError::Driver(format!("invalid GB28181 config: {e}")))?;

    let secret = ctx.secret(&cfg.digest_secret_name).await?.ok_or_else(|| {
        PluginError::Driver(format!("missing digest secret: {}", cfg.digest_secret_name))
    })?;

    let digest_bytes = hex::decode(secret.expose_secret())
        .map_err(|e| PluginError::Driver(format!("digest secret is not valid hex: {e}")))?;
    let digest_secret = SecretSlice::from(digest_bytes);
    if digest_secret.expose_secret().len() < 32 {
        return Err(PluginError::Driver(
            "digest_secret must be at least 32 bytes".to_string(),
        ));
    }

    let domain_config = Gb28181DomainConfig::new(&cfg.domain_id, &cfg.realm, digest_secret)
        .map_err(|e| PluginError::Driver(format!("GB28181 domain config: {e}")))?
        .with_auth_policy(AuthPolicy::ChallengeOptional)
        .with_allow_md5(cfg.allow_md5.unwrap_or(false))
        .with_preferred_algorithm(parse_algorithm(&cfg.preferred_algorithm)?);

    Ok(domain_config)
}

fn parse_algorithm(
    s: &Option<String>,
) -> Result<cheetah_gb28181_core::DigestAlgorithm, PluginError> {
    match s.as_deref() {
        None | Some("sha256") => Ok(cheetah_gb28181_core::DigestAlgorithm::Sha256),
        Some("sha512") => Ok(cheetah_gb28181_core::DigestAlgorithm::Sha512),
        Some("md5") => Ok(cheetah_gb28181_core::DigestAlgorithm::Md5),
        Some(other) => Err(PluginError::Unsupported(format!(
            "unsupported digest algorithm {other}"
        ))),
    }
}

#[derive(Deserialize)]
struct DriverConfig {
    domain_id: String,
    realm: String,
    digest_secret_name: String,
    #[serde(default)]
    allow_md5: Option<bool>,
    #[serde(default)]
    preferred_algorithm: Option<String>,
}

#[derive(Deserialize)]
struct ProcessSipPayload {
    source: String,
    message_hex: String,
}

async fn process_sip(
    driver: &Gb28181ProtocolDriver,
    ctx: &dyn DriverContext,
    payload: &serde_json::Value,
) -> Result<(), PluginError> {
    let req: ProcessSipPayload = serde_json::from_value(payload.clone())
        .map_err(|e| PluginError::Driver(format!("process_sip payload: {e}")))?;
    let source = req
        .source
        .parse::<SocketAddr>()
        .map_err(|e| PluginError::Driver(format!("invalid source address: {e}")))?;
    let bytes = hex::decode(&req.message_hex)
        .map_err(|e| PluginError::Driver(format!("invalid message_hex: {e}")))?;

    let message = SipParser::parse_datagram(&bytes, SipParserConfig::default())
        .map_err(|e| PluginError::Driver(format!("SIP parse error: {e}")))?;

    let now = ctx.monotonic_now();
    let input = AccessInput {
        source,
        now,
        message,
    };

    let outputs = {
        let mut guard = driver.inner.lock().await;
        let access = guard
            .as_mut()
            .ok_or_else(|| PluginError::Driver("GB28181 driver is not started".to_string()))?;

        access
            .process(input)
            .map_err(|e| PluginError::Driver(format!("GB28181 access error: {e}")))?
    };

    for output in outputs {
        let event = match output {
            AccessOutput::SendResponse(message) => ProtocolEvent {
                event_type: "gb28181.sip.response".to_string(),
                payload: serde_json::json!({
                    "destination": source.to_string(),
                    "message_hex": hex::encode(encode_message(&message)),
                }),
                tenant_id: None,
            },
            AccessOutput::EmitEvent(gb_event) => gb28181_event_to_protocol(&gb_event),
        };
        ctx.device_sink()
            .emit_event(event)
            .await
            .map_err(|e| PluginError::Driver(format!("event sink error: {e}")))?;
    }

    Ok(())
}

fn gb28181_event_to_protocol(event: &Gb28181Event) -> ProtocolEvent {
    // Build a generic JSON payload from the event fields. The event enum does
    // not derive Serialize because it contains sensitive addresses and IDs that
    // may need redaction; we serialize only stable, non-secret fields here.
    let payload = match event {
        Gb28181Event::DeviceRegistered {
            domain_id,
            device_id,
            source,
            contact,
            expires,
            user_agent,
        } => serde_json::json!({
            "event": "device_registered",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "contact": contact,
            "expires": expires,
            "user_agent": user_agent,
        }),
        Gb28181Event::DeviceUnregistered {
            domain_id,
            device_id,
            source,
        } => serde_json::json!({
            "event": "device_unregistered",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
        }),
        Gb28181Event::DevicePresenceChanged {
            domain_id,
            device_id,
            source,
            presence,
        } => {
            serde_json::json!({
                "event": "device_presence_changed",
                "domain_id": domain_id.to_string(),
                "device_id": device_id.to_string(),
                "source": source.to_string(),
                "presence": format!("{presence:?}"),
            })
        }
        Gb28181Event::Keepalive {
            domain_id,
            device_id,
            source,
            status,
        } => serde_json::json!({
            "event": "keepalive",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "status": status,
        }),
        Gb28181Event::CatalogReceived {
            domain_id,
            device_id,
            source,
            sn,
            sum_num,
            num,
            items,
        } => serde_json::json!({
            "event": "catalog_received",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "sn": sn,
            "sum_num": sum_num,
            "num": num,
            "items": items.iter().map(|i| format!("{i:?}")).collect::<Vec<_>>(),
        }),
        Gb28181Event::DeviceInfoReceived {
            domain_id,
            device_id,
            source,
            sn,
            result,
            manufacturer,
            model,
            firmware,
        } => serde_json::json!({
            "event": "device_info_received",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "sn": sn,
            "result": result,
            "manufacturer": manufacturer,
            "model": model,
            "firmware": firmware,
        }),
        Gb28181Event::DeviceStatusReceived {
            domain_id,
            device_id,
            source,
            sn,
            result,
            online,
            status,
            reason,
            invalid_equip,
        } => serde_json::json!({
            "event": "device_status_received",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "sn": sn,
            "result": result,
            "online": online,
            "status": status,
            "reason": reason,
            "invalid_equip": invalid_equip,
        }),
        Gb28181Event::AlarmReceived {
            domain_id,
            device_id,
            source,
            sn,
            priority,
            method,
            alarm_type,
            time,
            info,
        } => serde_json::json!({
            "event": "alarm_received",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "sn": sn,
            "priority": priority,
            "method": method,
            "alarm_type": alarm_type,
            "time": time,
            "info": info,
        }),
        Gb28181Event::MobilePositionReceived {
            domain_id,
            device_id,
            source,
            sn,
            time,
            longitude,
            latitude,
            speed,
            direction,
            altitude,
        } => serde_json::json!({
            "event": "mobile_position_received",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "sn": sn,
            "time": time,
            "longitude": longitude,
            "latitude": latitude,
            "speed": speed,
            "direction": direction,
            "altitude": altitude,
        }),
        Gb28181Event::RecordInfoReceived {
            domain_id,
            device_id,
            source,
            sn,
            name,
            sum_num,
            num,
            items,
        } => serde_json::json!({
            "event": "record_info_received",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "sn": sn,
            "name": name,
            "sum_num": sum_num,
            "num": num,
            "items": items.iter().map(|i| format!("{i:?}")).collect::<Vec<_>>(),
        }),
        Gb28181Event::DeviceControlResponseReceived {
            domain_id,
            device_id,
            source,
            sn,
            result,
        } => serde_json::json!({
            "event": "device_control_response_received",
            "domain_id": domain_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "sn": sn,
            "result": result,
        }),
        Gb28181Event::MediaSessionStarted {
            domain_id,
            media_session_id,
            channel_id,
            device_id,
            source,
            remote_sdp,
            remote_ssrc,
            remote_port,
            remote_proto,
        } => serde_json::json!({
            "event": "media_session_started",
            "domain_id": domain_id.to_string(),
            "media_session_id": media_session_id.to_string(),
            "channel_id": channel_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
            "remote_sdp": remote_sdp,
            "remote_ssrc": remote_ssrc,
            "remote_port": remote_port,
            "remote_proto": remote_proto,
        }),
        Gb28181Event::MediaSessionStopped {
            domain_id,
            media_session_id,
            channel_id,
            device_id,
            source,
        } => serde_json::json!({
            "event": "media_session_stopped",
            "domain_id": domain_id.to_string(),
            "media_session_id": media_session_id.to_string(),
            "channel_id": channel_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.to_string(),
        }),
        Gb28181Event::MediaSessionFailed {
            domain_id,
            media_session_id,
            channel_id,
            device_id,
            source,
            reason,
        } => serde_json::json!({
            "event": "media_session_failed",
            "domain_id": domain_id.to_string(),
            "media_session_id": media_session_id.to_string(),
            "channel_id": channel_id.to_string(),
            "device_id": device_id.to_string(),
            "source": source.map(|s| s.to_string()),
            "reason": reason,
        }),
        Gb28181Event::CascadePlatformConnected {
            domain_id,
            platform_id,
            upstream,
            expires,
        } => serde_json::json!({
            "event": "cascade_platform_connected",
            "domain_id": domain_id.to_string(),
            "platform_id": platform_id,
            "upstream": upstream,
            "expires": expires,
        }),
        Gb28181Event::CascadePlatformDisconnected {
            domain_id,
            platform_id,
            reason,
        } => {
            serde_json::json!({
                "event": "cascade_platform_disconnected",
                "domain_id": domain_id.to_string(),
                "platform_id": platform_id,
                "reason": reason,
            })
        }
        Gb28181Event::CascadePlayRequested {
            domain_id,
            platform_id,
            bridge_id,
            upstream_call_id,
            upstream_from,
            upstream_to,
            target_user,
            remote_sdp,
        } => serde_json::json!({
            "event": "cascade_play_requested",
            "domain_id": domain_id.to_string(),
            "platform_id": platform_id,
            "bridge_id": bridge_id,
            "upstream_call_id": upstream_call_id,
            "upstream_from": upstream_from,
            "upstream_to": upstream_to,
            "target_user": target_user,
            "remote_sdp": remote_sdp,
        }),
        Gb28181Event::CascadePlayStopped {
            domain_id,
            platform_id,
            bridge_id,
            reason,
        } => serde_json::json!({
            "event": "cascade_play_stopped",
            "domain_id": domain_id.to_string(),
            "platform_id": platform_id,
            "bridge_id": bridge_id,
            "reason": reason,
        }),
    };

    ProtocolEvent {
        event_type: "gb28181.event".to_string(),
        payload,
        tenant_id: None,
    }
}

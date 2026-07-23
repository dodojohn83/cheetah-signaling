//! Event payload builders and emit helpers for the ONVIF Tokio driver.
//!
//! Each emitted event carries the originating command's idempotency key so the
//! host can correlate the asynchronous result with the command that produced it.

use cheetah_onvif_core::services::system_date_time::DateTime;
use cheetah_onvif_services::services::{
    MediaDialect, MediaProfile, OnvifNotification, PtzPreset, PullPointSubscription, SnapshotUri,
    StreamUri, SystemDateAndTime, normalize_topic, redact_uri_userinfo,
};
use cheetah_onvif_services::{CapabilityKind, CapabilityProbeResult, DeviceInformation, Service};
use cheetah_plugin_sdk::{DriverContext, PluginError, ProtocolEvent};
use cheetah_signal_types::{TenantId, UtcTimestamp};
use serde_json::{Value, json};
use std::collections::HashMap;

fn dialect_str(dialect: MediaDialect) -> &'static str {
    match dialect {
        MediaDialect::Media1 => "media1",
        MediaDialect::Media2 => "media2",
    }
}

fn date_time_value(dt: &DateTime) -> Value {
    match dt.to_utc() {
        Ok(utc) => match UtcTimestamp::from_offset(utc).to_rfc3339() {
            Ok(s) => Value::String(s),
            Err(_) => date_time_components_value(dt),
        },
        Err(_) => date_time_components_value(dt),
    }
}

/// Serializes a [`DateTime`] as a structured object without a timezone
/// designator. This is intended for local wall-clock times, which must not be
/// stamped with a UTC `Z` suffix.
fn date_time_components_value(dt: &DateTime) -> Value {
    json!({
        "year": dt.year,
        "month": dt.month,
        "day": dt.day,
        "hour": dt.hour,
        "minute": dt.minute,
        "second": dt.second,
    })
}

/// Builds a JSON payload for a snapshot URI query result.
pub(crate) fn snapshot_event_payload(
    profile_token: &str,
    uri: &SnapshotUri,
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "profile_token": profile_token,
        "uri": redact_uri_userinfo(&uri.uri),
        "invalid_after_connect": uri.invalid_after_connect,
        "invalid_after_reboot": uri.invalid_after_reboot,
        "timeout": uri.timeout,
    })
}

/// Emits an `onvif.snapshot_uri` event with a redacted URI.
pub(crate) async fn emit_snapshot(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    profile_token: &str,
    uri: &SnapshotUri,
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.snapshot_uri".into(),
            payload: snapshot_event_payload(profile_token, uri, idempotency_key),
            tenant_id,
        })
        .await
}

/// Builds a JSON payload for a stream URI query result.
pub(crate) fn stream_uri_event_payload(
    profile_token: &str,
    uri: &StreamUri,
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "profile_token": profile_token,
        "uri": redact_uri_userinfo(&uri.uri),
        "invalid_after_connect": uri.invalid_after_connect,
        "invalid_after_reboot": uri.invalid_after_reboot,
        "timeout": uri.timeout,
    })
}

/// Emits an `onvif.stream_uri` event with a redacted URI.
pub(crate) async fn emit_stream_uri(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    profile_token: &str,
    uri: &StreamUri,
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.stream_uri".into(),
            payload: stream_uri_event_payload(profile_token, uri, idempotency_key),
            tenant_id,
        })
        .await
}

/// Builds a JSON payload for a media profile query result.
pub(crate) fn profiles_event_payload(
    dialect: MediaDialect,
    profiles: &[MediaProfile],
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "dialect": dialect_str(dialect),
        "profiles": profiles.iter().map(|p| json!({
            "token": p.token,
            "name": p.name,
            "fixed": p.fixed,
        })).collect::<Vec<_>>(),
    })
}

/// Emits an `onvif.profiles` event.
pub(crate) async fn emit_profiles(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    dialect: MediaDialect,
    profiles: &[MediaProfile],
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.profiles".into(),
            payload: profiles_event_payload(dialect, profiles, idempotency_key),
            tenant_id,
        })
        .await
}

/// Builds a JSON payload for a device information query result.
pub(crate) fn device_information_event_payload(
    info: &DeviceInformation,
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "manufacturer": info.manufacturer,
        "model": info.model,
        "firmware_version": info.firmware_version,
        "serial_number": info.serial_number,
        "hardware_id": info.hardware_id,
    })
}

/// Emits an `onvif.device_information` event.
pub(crate) async fn emit_device_information(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    info: &DeviceInformation,
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.device_information".into(),
            payload: device_information_event_payload(info, idempotency_key),
            tenant_id,
        })
        .await
}

/// Builds a JSON payload for a system date and time query result.
pub(crate) fn system_date_and_time_event_payload(
    dt: &SystemDateAndTime,
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "date_time_type": dt.date_time_type,
        "daylight_savings": dt.daylight_savings,
        "timezone": dt.timezone,
        "utc": date_time_value(&dt.utc),
        "local": dt.local.as_ref().map(date_time_components_value),
    })
}

/// Emits an `onvif.system_date_and_time` event.
pub(crate) async fn emit_system_date_and_time(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    dt: &SystemDateAndTime,
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.system_date_and_time".into(),
            payload: system_date_and_time_event_payload(dt, idempotency_key),
            tenant_id,
        })
        .await
}

fn services_value(services: &[Service]) -> Value {
    Value::Array(
        services
            .iter()
            .map(|s| {
                json!({
                    "namespace": s.namespace,
                    "xaddr": redact_uri_userinfo(&s.xaddr),
                    "version": s.version,
                })
            })
            .collect(),
    )
}

/// Builds a JSON payload for a `GetServices` query result.
pub(crate) fn services_event_payload(services: &[Service], idempotency_key: &str) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "services": services_value(services),
    })
}

/// Serializes the service list to a compact JSON string (for probe metadata).
pub(crate) fn services_to_json(services: &[Service]) -> String {
    serde_json::to_string(&services_value(services)).unwrap_or_default()
}

/// Emits an `onvif.services` event.
pub(crate) async fn emit_services(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    services: &[Service],
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.services".into(),
            payload: services_event_payload(services, idempotency_key),
            tenant_id,
        })
        .await
}

fn capabilities_value(caps: &HashMap<CapabilityKind, CapabilityProbeResult>) -> Value {
    let mut map = serde_json::Map::new();
    for (kind, result) in caps {
        let value = match result {
            CapabilityProbeResult::Supported {
                namespace,
                xaddr,
                version,
            } => json!({
                "status": "supported",
                "namespace": namespace,
                "xaddr": xaddr.as_deref().map(redact_uri_userinfo),
                "version": version,
            }),
            CapabilityProbeResult::Unsupported => json!({"status": "unsupported"}),
            CapabilityProbeResult::Failed { reason, retryable } => json!({
                "status": "failed",
                "reason": reason,
                "retryable": retryable,
            }),
        };
        map.insert(kind.to_string(), value);
    }
    Value::Object(map)
}

/// Builds a JSON payload for a `GetCapabilities` query result.
pub(crate) fn capabilities_event_payload(
    caps: &HashMap<CapabilityKind, CapabilityProbeResult>,
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "capabilities": capabilities_value(caps),
    })
}

/// Serializes the capability map to a compact JSON string (for probe metadata).
pub(crate) fn capabilities_to_json(
    caps: &HashMap<CapabilityKind, CapabilityProbeResult>,
) -> String {
    serde_json::to_string(&capabilities_value(caps)).unwrap_or_default()
}

/// Emits an `onvif.capabilities` event.
pub(crate) async fn emit_capabilities(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    caps: &HashMap<CapabilityKind, CapabilityProbeResult>,
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.capabilities".into(),
            payload: capabilities_event_payload(caps, idempotency_key),
            tenant_id,
        })
        .await
}

/// Builds a JSON payload for a PTZ preset query result.
pub(crate) fn ptz_presets_event_payload(
    profile_token: &str,
    presets: &[PtzPreset],
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "profile_token": profile_token,
        "presets": presets.iter().map(|p| json!({"token": p.token, "name": p.name})).collect::<Vec<_>>(),
    })
}

/// Emits an `onvif.ptz_presets` event.
pub(crate) async fn emit_ptz_presets(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    profile_token: &str,
    presets: &[PtzPreset],
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.ptz_presets".into(),
            payload: ptz_presets_event_payload(profile_token, presets, idempotency_key),
            tenant_id,
        })
        .await
}

/// Builds a JSON payload for a batch of pull-point notifications.
pub(crate) fn notifications_event_payload(
    notifications: &[OnvifNotification],
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "notifications": notifications
            .iter()
            .map(|n| {
                json!({
                    "topic": normalize_topic(&n.topic),
                    "utc_time": n.utc_time,
                    "property_operation": n.property_operation,
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// Emits an `onvif.notification` event.
pub(crate) async fn emit_onvif_notifications(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    messages: Vec<OnvifNotification>,
    idempotency_key: &str,
) -> Result<(), PluginError> {
    if messages.is_empty() {
        return Ok(());
    }
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.notification".into(),
            payload: notifications_event_payload(&messages, idempotency_key),
            tenant_id,
        })
        .await
}

/// Builds a JSON payload for a pull-point subscription creation result.
pub(crate) fn pull_point_subscription_event_payload(
    sub: &PullPointSubscription,
    idempotency_key: &str,
) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "subscription_reference": redact_uri_userinfo(&sub.subscription_reference),
        "termination_time": sub.termination_time,
        "current_time": sub.current_time,
    })
}

/// Emits an `onvif.pull_point_subscription` event.
pub(crate) async fn emit_pull_point_subscription(
    ctx: &dyn DriverContext,
    tenant_id: Option<TenantId>,
    sub: &PullPointSubscription,
    idempotency_key: &str,
) -> Result<(), PluginError> {
    ctx.device_sink()
        .emit_event(ProtocolEvent {
            event_type: "onvif.pull_point_subscription".into(),
            payload: pull_point_subscription_event_payload(sub, idempotency_key),
            tenant_id,
        })
        .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn snapshot_event_payload_redacts_uri_userinfo_and_includes_idempotency_key() {
        let uri = SnapshotUri {
            uri: "http://user:pass@192.0.2.10/snapshot".into(),
            invalid_after_connect: Some(false),
            invalid_after_reboot: Some(false),
            timeout: None,
        };
        let payload = snapshot_event_payload("profile-1", &uri, "idem-1");
        let emitted = payload["uri"].as_str().unwrap();
        assert!(!emitted.contains("pass"));
        assert!(emitted.contains("192.0.2.10"));
        assert_eq!(payload["invalid_after_connect"].as_bool(), Some(false));
        assert_eq!(payload["profile_token"].as_str(), Some("profile-1"));
        assert_eq!(payload["idempotency_key"].as_str(), Some("idem-1"));
    }

    #[test]
    fn stream_uri_event_payload_redacts_uri_userinfo_and_includes_idempotency_key() {
        let uri = StreamUri {
            uri: "rtsp://admin:secret@192.0.2.20/stream".into(),
            invalid_after_connect: Some(false),
            invalid_after_reboot: Some(false),
            timeout: Some("PT30S".into()),
        };
        let payload = stream_uri_event_payload("profile-2", &uri, "idem-2");
        let emitted = payload["uri"].as_str().unwrap();
        assert!(!emitted.contains("secret"));
        assert!(emitted.contains("192.0.2.20"));
        assert_eq!(payload["profile_token"].as_str(), Some("profile-2"));
        assert_eq!(payload["idempotency_key"].as_str(), Some("idem-2"));
    }
}

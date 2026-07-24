//! GB28181 driver-event dispatch.
//!
//! [`process_event`] fans each [`Gb28181Event`] out to the device, catalog,
//! media-session and outbox helpers, building a per-event [`RequestContext`]
//! that carries the source IP for audit routing.

use cheetah_gb28181_module::Gb28181Event;
use cheetah_http_api::state::ApiState;
use cheetah_signal_types::{
    CorrelationId, GbCommandMethod, GbCommandOutcome, GbMetricsRecorder, MessageId, NodeId,
    Principal, PrincipalKind, RequestContext, SignalError, TenantId,
};
use std::collections::BTreeMap;
use tracing::warn;
use uuid::Uuid;

use super::catalog::replace_catalog;
use super::device::{
    ensure_online, mark_offline, resolve_device_id, submit_bootstrap_queries, update_device_info,
};
use super::media_session::{MediaSessionTransition, handle_media_session_event};
use super::outbox::{append_gb_event, serialize_record_items};
use crate::gb_catalog_buffer::{CatalogBuffer, RecordInfoBuffer};

#[allow(clippy::too_many_arguments)]
pub(super) async fn process_event(
    state: &ApiState,
    node_id: NodeId,
    tenant_id: Option<TenantId>,
    event: Gb28181Event,
    catalog_buffer: &mut CatalogBuffer,
    record_buffer: &mut RecordInfoBuffer,
    gb_metrics: &dyn GbMetricsRecorder,
) -> Result<(), SignalError> {
    let tenant_id = match tenant_id {
        Some(id) => id,
        None => {
            warn!("dropping gb28181 event; no default_tenant_id configured");
            return Ok(());
        }
    };

    let context = build_context(state, node_id, tenant_id, &event);

    match event {
        Gb28181Event::DeviceRegistered {
            device_id,
            registration_sequence,
            ..
        } => match ensure_online(state, &context, tenant_id, &device_id, true).await {
            Ok(Some(internal_id)) => {
                submit_bootstrap_queries(
                    state,
                    &context,
                    tenant_id,
                    internal_id,
                    registration_sequence,
                )
                .await
            }
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        },
        Gb28181Event::DeviceUnregistered { device_id, .. } => {
            mark_offline(state, &context, tenant_id, &device_id).await
        }
        Gb28181Event::DevicePresenceChanged {
            device_id,
            presence,
            ..
        } => match presence {
            cheetah_gb28181_module::DevicePresence::Online => {
                ensure_online(state, &context, tenant_id, &device_id, true)
                    .await
                    .map(|_| ())
            }
            cheetah_gb28181_module::DevicePresence::Offline => {
                mark_offline(state, &context, tenant_id, &device_id).await
            }
        },
        Gb28181Event::Keepalive { device_id, .. } => {
            ensure_online(state, &context, tenant_id, &device_id, false)
                .await
                .map(|_| ())
        }
        Gb28181Event::CatalogReceived {
            device_id,
            sn,
            sum_num,
            num,
            items,
            ..
        } => {
            gb_metrics.record_catalog_fragment();
            match catalog_buffer.accumulate(tenant_id, &device_id, &sn, sum_num, num, items) {
                Some(merged) => {
                    replace_catalog(state, &context, tenant_id, &device_id, &merged).await
                }
                None => Ok(()),
            }
        }
        Gb28181Event::DeviceInfoReceived {
            device_id,
            result,
            manufacturer,
            model,
            firmware,
            ..
        } => {
            let mut metadata = BTreeMap::new();
            if let Some(v) = result {
                metadata.insert("info_result".to_string(), v.clone());
            }
            if let Some(v) = manufacturer {
                metadata.insert("manufacturer".to_string(), v.clone());
            }
            if let Some(v) = model {
                metadata.insert("model".to_string(), v.clone());
            }
            if let Some(v) = firmware {
                metadata.insert("firmware".to_string(), v.clone());
            }
            update_device_info(state, &context, tenant_id, &device_id, metadata).await
        }
        Gb28181Event::DeviceStatusReceived {
            device_id,
            result,
            online,
            status,
            reason,
            invalid_equip,
            ..
        } => {
            let mut metadata = BTreeMap::new();
            if let Some(v) = result {
                metadata.insert("status_result".to_string(), v.clone());
            }
            if let Some(v) = online {
                metadata.insert("online".to_string(), v.clone());
            }
            if let Some(v) = status {
                metadata.insert("status".to_string(), v.clone());
            }
            if let Some(v) = reason {
                metadata.insert("reason".to_string(), v.clone());
            }
            if let Some(v) = invalid_equip {
                metadata.insert("invalid_equip".to_string(), v.clone());
            }
            update_device_info(state, &context, tenant_id, &device_id, metadata).await
        }
        Gb28181Event::AlarmReceived {
            device_id,
            sn,
            priority,
            method,
            alarm_type,
            time,
            info,
            ..
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = priority {
                payload.insert("priority".to_string(), v);
            }
            if let Some(v) = method {
                payload.insert("method".to_string(), v);
            }
            if let Some(v) = alarm_type {
                payload.insert("alarm_type".to_string(), v);
            }
            if let Some(v) = time {
                payload.insert("time".to_string(), v);
            }
            if let Some(v) = info {
                payload.insert("info".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "Alarm",
                payload,
            )
            .await
        }
        Gb28181Event::MobilePositionReceived {
            device_id,
            sn,
            time,
            longitude,
            latitude,
            speed,
            direction,
            altitude,
            ..
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = time {
                payload.insert("time".to_string(), v);
            }
            if let Some(v) = longitude {
                payload.insert("longitude".to_string(), v);
            }
            if let Some(v) = latitude {
                payload.insert("latitude".to_string(), v);
            }
            if let Some(v) = speed {
                payload.insert("speed".to_string(), v);
            }
            if let Some(v) = direction {
                payload.insert("direction".to_string(), v);
            }
            if let Some(v) = altitude {
                payload.insert("altitude".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "MobilePosition",
                payload,
            )
            .await
        }
        Gb28181Event::DeviceControlResponseReceived {
            device_id,
            sn,
            result,
            ..
        } => {
            gb_metrics.record_command(GbCommandMethod::DeviceControl, control_outcome(&result));
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = result {
                payload.insert("result".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "DeviceControl",
                payload,
            )
            .await
        }
        Gb28181Event::MediaStatusReceived {
            device_id,
            sn,
            notify_type,
            outcome,
            ..
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            payload.insert("notify_type".to_string(), notify_type);
            payload.insert("outcome".to_string(), format!("{outcome:?}"));
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "MediaStatus",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionStarted {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
            remote_ssrc,
            remote_port,
            remote_proto,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            payload.insert("remote_address".to_string(), source.to_string());
            if let Some(v) = remote_ssrc {
                payload.insert("remote_ssrc".to_string(), v);
            }
            payload.insert("remote_port".to_string(), remote_port.to_string());
            payload.insert("remote_proto".to_string(), remote_proto);
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Start,
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionStarted",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionStopped {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            if let Some(s) = source {
                payload.insert("remote_address".to_string(), s.to_string());
            }
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Stop,
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionStopped",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionFailed {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
            reason,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            if let Some(s) = source {
                payload.insert("remote_address".to_string(), s.to_string());
            }
            payload.insert("reason".to_string(), reason.clone());
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Fail(reason),
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionFailed",
                payload,
            )
            .await
        }
        Gb28181Event::RecordInfoReceived {
            device_id,
            sn,
            num,
            sum_num,
            items,
            ..
        } => {
            if let Some(records) =
                record_buffer.accumulate(tenant_id, &device_id, &sn, sum_num, num, items)
            {
                let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
                let mut payload = BTreeMap::new();
                payload.insert("sn".to_string(), sn);
                payload.insert("sum_num".to_string(), sum_num.to_string());
                payload.insert("num".to_string(), num.to_string());
                payload.insert("record_count".to_string(), records.len().to_string());
                if !records.is_empty()
                    && let Some(json) = serialize_record_items(&records)
                {
                    payload.insert("records".to_string(), json);
                }
                append_gb_event(
                    state,
                    &context,
                    tenant_id,
                    internal_id,
                    Some(device_id.as_ref()),
                    "RecordInfo",
                    payload,
                )
                .await?;
            }
            Ok(())
        }
        Gb28181Event::CascadePlatformConnected {
            domain_id,
            platform_id,
            upstream,
            expires,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("upstream".to_string(), upstream);
            payload.insert("expires".to_string(), expires.to_string());
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlatformConnected",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlatformDisconnected {
            domain_id,
            platform_id,
            reason,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("reason".to_string(), reason);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlatformDisconnected",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlayRequested {
            domain_id,
            platform_id,
            bridge_id,
            upstream_call_id,
            upstream_from,
            upstream_to,
            target_user,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("bridge_id".to_string(), bridge_id.clone());
            payload.insert("upstream_call_id".to_string(), upstream_call_id);
            payload.insert("upstream_from".to_string(), upstream_from);
            payload.insert("upstream_to".to_string(), upstream_to);
            payload.insert("target_user".to_string(), target_user);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlayRequested",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlayStopped {
            domain_id,
            platform_id,
            bridge_id,
            reason,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("bridge_id".to_string(), bridge_id.clone());
            payload.insert("reason".to_string(), reason);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlayStopped",
                payload,
            )
            .await
        }
    }
}

fn build_context(
    _state: &ApiState,
    node_id: NodeId,
    tenant_id: TenantId,
    event: &Gb28181Event,
) -> RequestContext {
    let source_ip = event_source(event).map(|s| s.ip().to_string());
    RequestContext {
        tenant_id,
        principal: Principal {
            id: "gb28181".to_string(),
            kind: PrincipalKind::Service,
            scopes: vec!["device:write".to_string()],
        },
        message_id: MessageId::from_uuid(Uuid::now_v7()),
        correlation_id: CorrelationId::from_uuid(Uuid::now_v7()),
        traceparent: None,
        tracestate: None,
        deadline: None,
        node_id: Some(node_id),
        source_ip,
    }
}

fn event_source(event: &Gb28181Event) -> Option<&std::net::SocketAddr> {
    match event {
        Gb28181Event::DeviceRegistered { source, .. } => Some(source),
        Gb28181Event::DeviceUnregistered { source, .. } => Some(source),
        Gb28181Event::DevicePresenceChanged { source, .. } => Some(source),
        Gb28181Event::Keepalive { source, .. } => Some(source),
        Gb28181Event::CatalogReceived { source, .. } => Some(source),
        Gb28181Event::DeviceInfoReceived { source, .. } => Some(source),
        Gb28181Event::DeviceStatusReceived { source, .. } => Some(source),
        Gb28181Event::AlarmReceived { source, .. } => Some(source),
        Gb28181Event::MobilePositionReceived { source, .. } => Some(source),
        Gb28181Event::DeviceControlResponseReceived { source, .. } => Some(source),
        Gb28181Event::MediaStatusReceived { source, .. } => Some(source),
        Gb28181Event::MediaSessionStarted { source, .. } => Some(source),
        Gb28181Event::MediaSessionStopped { source, .. } => source.as_ref(),
        Gb28181Event::MediaSessionFailed { source, .. } => source.as_ref(),
        Gb28181Event::RecordInfoReceived { source, .. } => Some(source),
        _ => None,
    }
}

/// Maps a GB28181 DeviceControl response result string to a bounded outcome.
fn control_outcome(result: &Option<String>) -> GbCommandOutcome {
    match result {
        Some(value) if value.eq_ignore_ascii_case("OK") => GbCommandOutcome::Succeeded,
        Some(_) => GbCommandOutcome::Failed,
        None => GbCommandOutcome::Unknown,
    }
}

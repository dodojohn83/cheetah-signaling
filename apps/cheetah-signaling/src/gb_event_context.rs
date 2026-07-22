//! GB28181 event processing context helpers.
//!
//! Small, stateless helpers for building `RequestContext`s, extracting event
//! source addresses, mapping command outcomes, and serializing record items.

use cheetah_gb28181_module::Gb28181Event;
use cheetah_gb28181_module::xml::RecordItem as GbRecordItem;
use cheetah_http_api::state::ApiState;
use cheetah_signal_types::{
    CorrelationId, GbCommandOutcome, MessageId, NodeId, Principal, PrincipalKind, RequestContext,
    TenantId,
};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use uuid::Uuid;

/// Builds a `RequestContext` for a GB28181 driver event.
pub(crate) fn build_context(
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

/// Returns the source socket address carried by a GB28181 driver event, if any.
pub(crate) fn event_source(event: &Gb28181Event) -> Option<&SocketAddr> {
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
        Gb28181Event::MediaSessionStarted { source, .. } => Some(source),
        Gb28181Event::MediaSessionStopped { source, .. } => source.as_ref(),
        Gb28181Event::MediaSessionFailed { source, .. } => source.as_ref(),
        Gb28181Event::RecordInfoReceived { source, .. } => Some(source),
        _ => None,
    }
}

/// Maps a raw command result string to a `GbCommandOutcome`.
pub(crate) fn control_outcome(result: &Option<String>) -> GbCommandOutcome {
    match result {
        Some(value) if value.eq_ignore_ascii_case("OK") => GbCommandOutcome::Succeeded,
        Some(_) => GbCommandOutcome::Failed,
        None => GbCommandOutcome::Unknown,
    }
}

/// Serializes a slice of GB28181 record items to a JSON array.
pub(crate) fn serialize_record_items(records: &[GbRecordItem]) -> Option<String> {
    let maps: Vec<BTreeMap<String, String>> = records
        .iter()
        .map(|r| {
            let mut m = BTreeMap::new();
            m.insert("device_id".to_string(), r.device_id.clone());
            if let Some(v) = &r.name {
                m.insert("name".to_string(), v.clone());
            }
            if let Some(v) = &r.file_path {
                m.insert("file_path".to_string(), v.clone());
            }
            if let Some(v) = &r.start_time {
                m.insert("start_time".to_string(), v.clone());
            }
            if let Some(v) = &r.end_time {
                m.insert("end_time".to_string(), v.clone());
            }
            if let Some(v) = &r.secrecy {
                m.insert("secrecy".to_string(), v.clone());
            }
            if let Some(v) = &r.record_type {
                m.insert("record_type".to_string(), v.clone());
            }
            if let Some(v) = &r.recorder_id {
                m.insert("recorder_id".to_string(), v.clone());
            }
            if let Some(v) = &r.file_size {
                m.insert("file_size".to_string(), v.clone());
            }
            m
        })
        .collect();
    serde_json::to_string(&maps).ok()
}

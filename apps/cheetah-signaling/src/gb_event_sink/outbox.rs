//! `Gb28181EventReceived` outbox envelope helpers.

use cheetah_domain::{DomainEvent, Protocol};
use cheetah_gb28181_module::xml::RecordItem as GbRecordItem;
use cheetah_http_api::state::ApiState;
use cheetah_signal_types::{
    DeviceId, Event, RequestContext, ResourceId, ResourceKind, ResourceRef, SignalError, TenantId,
};
use std::collections::BTreeMap;

use super::storage_error;

/// Builds a [`DomainEvent::Gb28181EventReceived`] outbox event.
///
/// When an internal device identifier is known the event is attached to the
/// device aggregate; otherwise it is attached to a synthetic event aggregate.
pub(super) fn build_gb_event(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: Option<DeviceId>,
    external_id: Option<&str>,
    event_type: &str,
    payload: BTreeMap<String, String>,
) -> Event<DomainEvent> {
    let event_id = state.id_generator.generate_event_id();
    let aggregate_ref = match device_id {
        Some(id) => ResourceRef {
            tenant_id,
            kind: ResourceKind::Device,
            id: ResourceId::Device(id),
        },
        None => ResourceRef {
            tenant_id,
            kind: ResourceKind::Event,
            id: ResourceId::Event(event_id),
        },
    };
    Event {
        event_id,
        tenant_id,
        aggregate_ref,
        aggregate_sequence: 0,
        occurred_at: state.clock.now_wall(),
        correlation_id: context.correlation_id,
        causation_id: context.message_id,
        traceparent: context.traceparent.clone(),
        tracestate: context.tracestate.clone(),
        source: context.node_id.unwrap_or_default(),
        payload: DomainEvent::Gb28181EventReceived {
            tenant_id,
            device_id,
            event_type: event_type.to_string(),
            protocol: Protocol::Gb28181,
            external_id: external_id.map(String::from),
            payload,
        },
    }
}

/// Appends a [`DomainEvent::Gb28181EventReceived`] to the outbox.
pub(super) async fn append_gb_event(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: Option<DeviceId>,
    external_id: Option<&str>,
    event_type: &str,
    payload: BTreeMap<String, String>,
) -> Result<(), SignalError> {
    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let event = build_gb_event(
        state,
        context,
        tenant_id,
        device_id,
        external_id,
        event_type,
        payload,
    );
    uow.outbox().append(event).await?;
    uow.commit().await?;
    Ok(())
}

/// Serializes a slice of GB28181 record items to a JSON array.
pub(super) fn serialize_record_items(records: &[GbRecordItem]) -> Option<String> {
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

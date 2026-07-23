//! Map domain [`Command`]s and [`Event`]s to proto envelopes.

use cheetah_domain::{Command, CommandPayload, DomainEvent};
use cheetah_signal_contracts::cheetah::common::v1 as proto;
use cheetah_signal_contracts::cheetah::control::v1 as control;
use cheetah_signal_types::{
    Event, ResourceId, ResourceRef, UtcTimestamp, validate_traceparent, validate_tracestate,
};
use prost_types::Timestamp;

fn to_uuid(id: impl std::fmt::Display) -> proto::Uuid {
    proto::Uuid {
        value: id.to_string(),
    }
}

fn to_timestamp(value: UtcTimestamp) -> Timestamp {
    let offset = value.as_offset();
    Timestamp {
        seconds: offset.unix_timestamp(),
        nanos: i32::try_from(offset.nanosecond()).unwrap_or(0),
    }
}

fn resource_ref_to_proto(value: &ResourceRef) -> proto::ResourceRef {
    let resource_id = match &value.id {
        ResourceId::Tenant(id) => id.to_string(),
        ResourceId::Device(id) => id.to_string(),
        ResourceId::Channel(id) => id.to_string(),
        ResourceId::Endpoint(id) => id.to_string(),
        ResourceId::ProtocolSession(id) => id.to_string(),
        ResourceId::MediaSession(id) => id.to_string(),
        ResourceId::Operation(id) => id.to_string(),
        ResourceId::Event(id) => id.to_string(),
        ResourceId::Plugin(id) => id.to_string(),
        ResourceId::Node(id) => id.to_string(),
        ResourceId::MediaBinding(id) => id.to_string(),
        ResourceId::MediaNode(id) => id.to_string(),
        _ => String::new(),
    };
    let kind = match value.kind {
        cheetah_signal_types::ResourceKind::Tenant => proto::ResourceKind::Tenant as i32,
        cheetah_signal_types::ResourceKind::Device => proto::ResourceKind::Device as i32,
        cheetah_signal_types::ResourceKind::Channel => proto::ResourceKind::Channel as i32,
        cheetah_signal_types::ResourceKind::Endpoint => proto::ResourceKind::Endpoint as i32,
        cheetah_signal_types::ResourceKind::ProtocolSession => {
            proto::ResourceKind::ProtocolSession as i32
        }
        cheetah_signal_types::ResourceKind::MediaSession => {
            proto::ResourceKind::MediaSession as i32
        }
        cheetah_signal_types::ResourceKind::Operation => proto::ResourceKind::Operation as i32,
        cheetah_signal_types::ResourceKind::Event => proto::ResourceKind::Event as i32,
        cheetah_signal_types::ResourceKind::Plugin => proto::ResourceKind::Plugin as i32,
        cheetah_signal_types::ResourceKind::Node => proto::ResourceKind::Node as i32,
        cheetah_signal_types::ResourceKind::MediaBinding => {
            proto::ResourceKind::MediaBinding as i32
        }
        cheetah_signal_types::ResourceKind::MediaNode => proto::ResourceKind::MediaNode as i32,
        _ => proto::ResourceKind::Unspecified as i32,
    };

    proto::ResourceRef {
        tenant_id: Some(to_uuid(value.tenant_id)),
        kind,
        resource_id: Some(to_uuid(resource_id)),
    }
}

fn channel_id(command: &Command) -> String {
    match command.payload() {
        CommandPayload::StartLive { channel_id, .. }
        | CommandPayload::StartPlayback { channel_id, .. }
        | CommandPayload::StartTalk { channel_id, .. }
        | CommandPayload::Ptz { channel_id, .. } => channel_id.to_string(),
        CommandPayload::StopMediaSession { .. } | CommandPayload::ControlPlayback { .. } => {
            String::new()
        }
        _ => String::new(),
    }
}

/// Encode a domain [`Command`] as a proto [`CommandEnvelope`].
///
/// The command is serialized to JSON and carried inside a
/// [`ControlCommand::ChannelCommand`] envelope. This preserves the proto
/// envelope boundary while avoiding a one-to-one proto message for every
/// domain command variant.
pub fn encode_command(command: &Command) -> Result<proto::CommandEnvelope, super::BusError> {
    let payload = serde_json::to_vec(command)?;
    let channel_command = control::ChannelCommand {
        device_id: command.device_id().to_string(),
        channel_id: channel_id(command),
        command_type: command.kind().to_string(),
        payload,
        detail: None,
    };

    let control_command = control::ControlCommand {
        command: Some(control::control_command::Command::ChannelCommand(
            channel_command,
        )),
    };

    Ok(proto::CommandEnvelope {
        meta: Some(proto::EnvelopeMeta {
            message_id: Some(to_uuid(command.message_id())),
            tenant_id: Some(to_uuid(command.tenant_id())),
            correlation_id: Some(to_uuid(command.correlation_id())),
            causation_id: Some(to_uuid(command.causation_id())),
            occurred_at: None,
            deadline: command.deadline().map(|d| to_timestamp(d.as_timestamp())),
            source_node_id: None,
            owner_epoch: command.expected_owner_epoch().0,
            traceparent: command.traceparent().unwrap_or("").to_string(),
            tracestate: command.tracestate().unwrap_or("").to_string(),
            contract_version: 0,
        }),
        target: Some(resource_ref_to_proto(command.target())),
        idempotency_key: command.idempotency_key().to_string(),
        operation_id: command.operation_id().to_string(),
        step_id: String::new(),
        command: Some(proto::command_envelope::Command::ControlCommand(
            control_command,
        )),
    })
}

/// Decode a proto [`CommandEnvelope`] into a domain [`Command`].
///
/// The command body is recovered from the [`ChannelCommand`] payload.
pub fn decode_command(envelope: &proto::CommandEnvelope) -> Result<Command, super::BusError> {
    let channel = match &envelope.command {
        Some(proto::command_envelope::Command::ControlCommand(control::ControlCommand {
            command: Some(control::control_command::Command::ChannelCommand(channel)),
        })) => channel,
        _ => {
            return Err(super::BusError::UnsupportedEnvelope(
                "command envelope must carry a ChannelCommand".to_string(),
            ));
        }
    };

    serde_json::from_slice(&channel.payload).map_err(super::BusError::Serialize)
}

/// Encode a domain [`Event<DomainEvent>`] as a proto [`EventEnvelope`].
///
/// The whole event is serialized to JSON and carried inside a [`GenericEvent`].
pub fn encode_event(event: &Event<DomainEvent>) -> Result<proto::EventEnvelope, super::BusError> {
    let payload = serde_json::to_vec(event)?;

    Ok(proto::EventEnvelope {
        meta: Some(proto::EnvelopeMeta {
            message_id: Some(to_uuid(event.event_id)),
            tenant_id: Some(to_uuid(event.tenant_id)),
            correlation_id: Some(to_uuid(event.correlation_id)),
            causation_id: Some(to_uuid(event.causation_id)),
            occurred_at: Some(to_timestamp(event.occurred_at)),
            deadline: None,
            source_node_id: Some(to_uuid(event.source)),
            owner_epoch: 0,
            traceparent: event.traceparent.clone().unwrap_or_default(),
            tracestate: event.tracestate.clone().unwrap_or_default(),
            contract_version: 0,
        }),
        aggregate: Some(resource_ref_to_proto(&event.aggregate_ref)),
        aggregate_sequence: event.aggregate_sequence,
        event: Some(proto::event_envelope::Event::GenericEvent(
            proto::GenericEvent {
                event_type: "domain_event".to_string(),
                payload,
            },
        )),
    })
}

/// Decode a proto [`EventEnvelope`] into a domain [`Event<DomainEvent>`].
pub fn decode_event(
    envelope: &proto::EventEnvelope,
) -> Result<Event<DomainEvent>, super::BusError> {
    let generic = match &envelope.event {
        Some(proto::event_envelope::Event::GenericEvent(event)) => event,
        _ => {
            return Err(super::BusError::UnsupportedEnvelope(
                "event envelope must carry a GenericEvent".to_string(),
            ));
        }
    };

    let mut event: Event<DomainEvent> =
        serde_json::from_slice(&generic.payload).map_err(super::BusError::Serialize)?;
    if event.traceparent.is_none()
        && let Some(meta) = &envelope.meta
        && validate_traceparent(&meta.traceparent).is_some()
    {
        event.traceparent = Some(meta.traceparent.clone());
    }
    if event.tracestate.is_none()
        && let Some(meta) = &envelope.meta
        && validate_tracestate(&meta.tracestate).is_some()
    {
        event.tracestate = Some(meta.tracestate.clone());
    }
    Ok(event)
}

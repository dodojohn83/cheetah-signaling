//! Integration tests for generated protobuf contracts.

use cheetah_signal_contracts::cheetah::cluster::v1::NodeStatus;
use cheetah_signal_contracts::cheetah::common::v1::{
    CommandEnvelope, CommandResult, CommandStatus, EnvelopeMeta, ErrorStatus, EventEnvelope,
    NodeCommandExecuteRequest, NodeCommandExecuteResponse, PageInfo, PageRequest, ResourceKind,
    ResourceRef, Uuid, command_envelope, event_envelope,
};
use cheetah_signal_contracts::cheetah::control::v1::{
    ControlCommand, ControlCommandResult, InviteDevice, control_command,
};
use cheetah_signal_contracts::cheetah::device::v1::{
    Device, DeviceCommand, DeviceEvent, DeviceStatus, MediaSessionStarted, device_event,
};
use prost::Message;

fn uuid(value: &str) -> Uuid {
    Uuid {
        value: value.to_string(),
    }
}

fn envelope_meta() -> EnvelopeMeta {
    EnvelopeMeta {
        message_id: Some(uuid("00000000-0000-0000-0000-000000000001")),
        tenant_id: Some(uuid("00000000-0000-0000-0000-000000000002")),
        correlation_id: Some(uuid("00000000-0000-0000-0000-000000000003")),
        causation_id: Some(uuid("00000000-0000-0000-0000-000000000004")),
        occurred_at: None,
        deadline: None,
        source_node_id: Some(uuid("00000000-0000-0000-0000-000000000005")),
        owner_epoch: 1,
        traceparent: "traceparent".to_string(),
        tracestate: "tracestate".to_string(),
    }
}

fn resource_ref(kind: ResourceKind) -> ResourceRef {
    ResourceRef {
        tenant_id: Some(uuid("00000000-0000-0000-0000-000000000002")),
        kind: kind as i32,
        resource_id: Some(uuid("00000000-0000-0000-0000-000000000006")),
    }
}

#[test]
fn command_envelope_roundtrip_with_control_command() {
    let original = CommandEnvelope {
        meta: Some(envelope_meta()),
        target: Some(resource_ref(ResourceKind::Device)),
        idempotency_key: "idempotency-key-1".to_string(),
        command: Some(command_envelope::Command::ControlCommand(ControlCommand {
            command: Some(control_command::Command::InviteDevice(InviteDevice {
                device_id: "device-1".to_string(),
                channel_id: "channel-1".to_string(),
                media_node_id: "node-1".to_string(),
                invite_timeout_ms: 30000,
            })),
        })),
    };

    let bytes = original.encode_to_vec();
    let decoded = CommandEnvelope::decode(&bytes[..])
        .unwrap_or_else(|e| panic!("decode command envelope: {e}"));

    assert_eq!(original, decoded);
}

#[test]
fn event_envelope_roundtrip_with_device_event() {
    let original = EventEnvelope {
        meta: Some(envelope_meta()),
        aggregate: Some(resource_ref(ResourceKind::Device)),
        aggregate_sequence: 42,
        event: Some(event_envelope::Event::DeviceEvent(DeviceEvent {
            event: Some(device_event::Event::MediaSessionStarted(
                MediaSessionStarted {
                    device_id: "device-1".to_string(),
                    channel_id: "channel-1".to_string(),
                    media_session_id: "session-1".to_string(),
                },
            )),
        })),
    };

    let bytes = original.encode_to_vec();
    let decoded =
        EventEnvelope::decode(&bytes[..]).unwrap_or_else(|e| panic!("decode event envelope: {e}"));

    assert_eq!(original, decoded);
}

#[test]
fn node_command_request_roundtrip() {
    let original = NodeCommandExecuteRequest {
        command: Some(CommandEnvelope {
            meta: Some(envelope_meta()),
            target: Some(resource_ref(ResourceKind::Device)),
            idempotency_key: "idempotent-key".to_string(),
            command: Some(command_envelope::Command::DeviceCommand(
                DeviceCommand::default(),
            )),
        }),
    };

    let bytes = original.encode_to_vec();
    let decoded = NodeCommandExecuteRequest::decode(&bytes[..])
        .unwrap_or_else(|e| panic!("decode request: {e}"));

    assert_eq!(original, decoded);
}

#[test]
fn command_result_roundtrip() {
    let original = NodeCommandExecuteResponse {
        result: Some(CommandResult {
            status: CommandStatus::Completed as i32,
            operation_id: "operation-1".to_string(),
            error: Some(ErrorStatus {
                code: "ERROR".to_string(),
                message: "something went wrong".to_string(),
                retryable: true,
                violations: Vec::new(),
            }),
        }),
    };

    let bytes = original.encode_to_vec();
    let decoded = NodeCommandExecuteResponse::decode(&bytes[..])
        .unwrap_or_else(|e| panic!("decode response: {e}"));

    assert_eq!(original, decoded);
}

#[test]
fn page_and_resource_ref_roundtrip() {
    let original = PageRequest {
        cursor: "cursor".to_string(),
        page_size: 50,
    };

    let bytes = original.encode_to_vec();
    let decoded =
        PageRequest::decode(&bytes[..]).unwrap_or_else(|e| panic!("decode page request: {e}"));
    assert_eq!(original, decoded);

    let info = PageInfo {
        total: 100,
        next_cursor: "next".to_string(),
        page_size: 50,
    };
    let bytes = info.encode_to_vec();
    let decoded = PageInfo::decode(&bytes[..]).unwrap_or_else(|e| panic!("decode page info: {e}"));
    assert_eq!(info, decoded);
}

#[test]
fn device_status_enum_values_match_schema() {
    assert_eq!(DeviceStatus::Unspecified as i32, 0);
    assert_eq!(DeviceStatus::Online as i32, 1);
    assert_eq!(DeviceStatus::Offline as i32, 2);
    assert_eq!(DeviceStatus::Busy as i32, 3);
    assert_eq!(DeviceStatus::Fault as i32, 4);
}

#[test]
fn node_status_enum_values_match_schema() {
    assert_eq!(NodeStatus::Unspecified as i32, 0);
    assert_eq!(NodeStatus::Active as i32, 1);
    assert_eq!(NodeStatus::Draining as i32, 2);
    assert_eq!(NodeStatus::Left as i32, 3);
}

#[test]
fn command_status_enum_values_match_schema() {
    assert_eq!(CommandStatus::Unspecified as i32, 0);
    assert_eq!(CommandStatus::Accepted as i32, 1);
    assert_eq!(CommandStatus::Completed as i32, 2);
    assert_eq!(CommandStatus::Failed as i32, 3);
    assert_eq!(CommandStatus::Rejected as i32, 4);
    assert_eq!(CommandStatus::Timeout as i32, 5);
}

#[test]
fn device_snapshot_roundtrip() {
    let original = Device {
        device_id: "device-1".to_string(),
        tenant_id: "tenant-1".to_string(),
        protocol_identity: "sip:device-1@example.com".to_string(),
        status: DeviceStatus::Online as i32,
        sip_id: "sip-1".to_string(),
        onvif_uuid: "onvif-1".to_string(),
        metadata: [("key".to_string(), "value".to_string())]
            .into_iter()
            .collect(),
        revision: 1,
        owner_epoch: 1,
        owner_node_id: "node-1".to_string(),
        created_at: None,
        updated_at: None,
    };

    let bytes = original.encode_to_vec();
    let decoded =
        Device::decode(&bytes[..]).unwrap_or_else(|e| panic!("decode device snapshot: {e}"));

    assert_eq!(original, decoded);
}

#[test]
fn control_command_result_roundtrip() {
    let original = ControlCommandResult {
        success: true,
        error_code: "".to_string(),
        error_message: "".to_string(),
    };

    let bytes = original.encode_to_vec();
    let decoded = ControlCommandResult::decode(&bytes[..])
        .unwrap_or_else(|e| panic!("decode command result: {e}"));

    assert_eq!(original, decoded);
}

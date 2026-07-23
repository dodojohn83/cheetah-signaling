//! Explicit domain-to-proto and proto-to-domain mapping for the media client.

use crate::client::{MediaControlRequest, MediaListSessionsRequest};
use cheetah_domain::{
    CommandPayload, DomainError, MediaNodeCallback, MediaNodeCallbackKind, MediaNodeCommand,
    MediaNodeCommandResult, MediaNodeEvent, MediaNodeSessionRef,
};
use cheetah_signal_grpc::cheetah::common::v1::{CommandStatus, MediaControlExecuteResponse};
use cheetah_signal_grpc::cheetah::media::v1::media_event::Event as MediaEventPayload;
use cheetah_signal_grpc::cheetah::media::v1::{
    MediaCommand, MediaControlPayload, MediaEvent, MediaMutationContext, MediaSessionRef,
    SubscribeRequest, media_command,
};
use cheetah_signal_types::{
    MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, OperationId,
    OwnerEpoch, Revision, TenantId, UtcTimestamp,
};
use std::str::FromStr;

/// Builds a typed `MediaControlRequest` from a domain command.
pub(crate) fn build_media_control_request(
    command: &MediaNodeCommand,
) -> Result<MediaControlRequest, DomainError> {
    let mut proto_command = map_command_to_media_command(command)?;
    proto_command.context = Some(MediaMutationContext {
        tenant_id: command.tenant_id.to_string(),
        request_id: command.request_id.clone(),
        correlation_id: command.request_id.clone(),
        message_id: MessageId::generate().to_string(),
        idempotency_key: command.idempotency_key.clone(),
        deadline: command
            .deadline
            .map(|d| d.as_timestamp().to_prost_timestamp()),
        source_signaling_node_id: command.source_node_id.to_string(),
        owner_epoch: command.owner_epoch.0,
        target_media_node_id: command.media_node_id.to_string(),
        target_media_node_instance_epoch: command.media_node_instance_epoch.0,
        operation_id: command.operation_id.to_string(),
        operation_step_id: command.payload.kind().to_string(),
        media_session_id: Some(command.media_session_id.to_string()),
        media_binding_id: Some(command.media_binding_id.to_string()),
        contract_version: command.contract_version as u64,
        traceparent: None,
        tracestate: None,
    });

    Ok(MediaControlRequest {
        request_id: command.request_id.clone(),
        tenant_id: command.tenant_id,
        media_session_id: command.media_session_id,
        media_binding_id: command.media_binding_id,
        operation_id: command.operation_id,
        owner_epoch: command.owner_epoch,
        source_node_id: command.source_node_id,
        media_node_id: command.media_node_id,
        target_media_node_instance_epoch: command.media_node_instance_epoch,
        deadline: command.deadline.map(|d| d.as_timestamp()),
        idempotency_key: command.idempotency_key.clone(),
        contract_version: command.contract_version,
        command: proto_command,
    })
}

/// Maps a typed domain media command to a typed proto `MediaCommand`.
pub fn map_command_to_media_command(
    command: &MediaNodeCommand,
) -> Result<MediaCommand, DomainError> {
    let target_media_node_instance_epoch = command.media_node_instance_epoch.0;

    let command = match &command.payload {
        CommandPayload::StopMediaSession { media_session_id } => {
            let media_session_id = media_session_id.to_string();
            media_command::Command::StopRtp(MediaSessionRef {
                media_session_id,
                device_id: String::new(),
                channel_id: String::new(),
                media_node_instance_epoch: target_media_node_instance_epoch,
            })
        }
        _ => {
            let payload = serde_json::to_vec(&command.payload).map_err(|e| {
                DomainError::invalid_argument(format!(
                    "failed to serialize media command payload ({}): {e}",
                    command.payload.kind()
                ))
            })?;
            media_command::Command::Control(MediaControlPayload {
                media_session_id: command.media_session_id.to_string(),
                command_type: command.payload.kind().to_string(),
                payload,
            })
        }
    };

    Ok(MediaCommand {
        command: Some(command),
        target_media_node_instance_epoch,
        context: None,
    })
}

/// Builds a typed list-sessions request from domain values.
pub(crate) fn build_list_sessions_request(
    tenant_id: TenantId,
    media_node_id: NodeId,
    media_node_instance_epoch: MediaNodeInstanceEpoch,
    page: &cheetah_signal_types::PageRequest,
) -> MediaListSessionsRequest {
    MediaListSessionsRequest {
        media_node_id,
        media_node_instance_epoch,
        tenant_id,
        page_size: page.page_size,
        page_token: page.cursor.clone(),
    }
}

/// Builds a typed subscribe request from a domain subscription request.
pub(crate) fn build_subscribe_request(
    request: &cheetah_domain::MediaSubscriptionRequest,
) -> SubscribeRequest {
    let message_id = MessageId::generate().to_string();
    SubscribeRequest {
        context: Some(MediaMutationContext {
            tenant_id: request.tenant_id.map_or(String::new(), |t| t.to_string()),
            request_id: message_id.clone(),
            correlation_id: message_id,
            message_id: MessageId::generate().to_string(),
            idempotency_key: format!(
                "subscribe-{}-{}",
                request.media_node_id,
                MessageId::generate()
            ),
            deadline: None,
            source_signaling_node_id: request.source_node_id.to_string(),
            owner_epoch: 0,
            target_media_node_id: request.media_node_id.to_string(),
            target_media_node_instance_epoch: request.media_node_instance_epoch.0,
            operation_id: OperationId::generate().to_string(),
            operation_step_id: "subscribe".to_string(),
            media_session_id: None,
            media_binding_id: None,
            contract_version: request.contract_version as u64,
            traceparent: None,
            tracestate: None,
        }),
        media_session_ids: Vec::new(),
        resume_cursor: request.resume_cursor.clone(),
        max_batch_size: request.max_batch_size,
        filter: None,
    }
}

/// Maps a proto `CommandResult` to the domain `MediaNodeCommandResult`.
pub fn map_command_result(
    response: MediaControlExecuteResponse,
) -> Result<MediaNodeCommandResult, DomainError> {
    let result = response
        .result
        .ok_or_else(|| DomainError::unavailable("media node returned no command result"))?;

    match CommandStatus::try_from(result.status) {
        Ok(CommandStatus::Completed) => Ok(MediaNodeCommandResult::Completed),
        Ok(CommandStatus::Accepted) => Ok(MediaNodeCommandResult::Accepted),
        // A timeout means the media node did not confirm the outcome; the
        // side effect may or may not have been applied, so surface it as an
        // unknown outcome for the reconciler rather than a terminal failure.
        Ok(CommandStatus::Timeout) => Ok(MediaNodeCommandResult::UnknownOutcome {
            code: "timeout".to_string(),
            message: result
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "media node command timed out".to_string()),
        }),
        Ok(s) => Ok(MediaNodeCommandResult::Failed {
            code: format!("{s:?}"),
            message: result
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_default(),
        }),
        Err(_) => Ok(MediaNodeCommandResult::Failed {
            code: "unknown_status".to_string(),
            message: result
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_default(),
        }),
    }
}

/// Maps a proto `MediaSessionRef` to the domain `MediaNodeSessionRef`.
///
/// Empty optional fields are treated as `None`; non-empty malformed fields
/// surface as `InvalidArgument` errors with the offending field name so that
/// malformed list responses are reported as contract violations.
pub fn map_proto_session_ref(proto: &MediaSessionRef) -> Result<MediaNodeSessionRef, DomainError> {
    let media_session_id = parse_required_id(&proto.media_session_id, "media_session_id")?;
    let device_id = parse_optional_id(&proto.device_id, "device_id")?;
    let channel_id = parse_optional_id(&proto.channel_id, "channel_id")?;

    Ok(MediaNodeSessionRef {
        media_session_id,
        device_id,
        channel_id,
        media_node_instance_epoch: MediaNodeInstanceEpoch(proto.media_node_instance_epoch),
    })
}

/// Maps a proto `MediaEvent` to the domain `MediaNodeEvent`.
///
/// Malformed or unrecognised events are not treated as stream errors; instead
/// they are returned with `callback: None` so the consumer can log a diagnostic
/// and advance its cursor without redelivering the event indefinitely.
pub fn map_media_event(event: &MediaEvent) -> MediaNodeEvent {
    let tenant_id = parse_id_or_default::<TenantId>(&event.tenant_id);
    let event_id = event.event_id.clone();
    let correlation_id = event.correlation_id.clone();
    let sequence = event.sequence;
    let occurred_at = event
        .occurred_at
        .as_ref()
        .and_then(UtcTimestamp::from_prost_timestamp);
    let traceparent = if event.traceparent.is_empty() {
        None
    } else {
        Some(event.traceparent.clone())
    };
    let tracestate = if event.tracestate.is_empty() {
        None
    } else {
        Some(event.tracestate.clone())
    };

    let callback = try_build_callback(event);

    MediaNodeEvent {
        tenant_id,
        event_id,
        correlation_id,
        sequence,
        occurred_at,
        traceparent,
        tracestate,
        callback,
    }
}

fn try_build_callback(event: &MediaEvent) -> Option<MediaNodeCallback> {
    if event.event_id.is_empty() {
        return None;
    }

    let media_node_id = parse_id_or_default::<NodeId>(&event.media_node_id);
    if media_node_id.as_uuid().is_nil() {
        return None;
    }

    let media_session_id = parse_id_or_default::<MediaSessionId>(&event.media_session_id);
    if media_session_id.as_uuid().is_nil() {
        return None;
    }

    let media_binding_id = parse_id_or_default::<MediaBindingId>(&event.media_binding_id);
    if media_binding_id.as_uuid().is_nil() {
        return None;
    }

    let kind = map_event_payload(event).ok()?;

    let operation_id = parse_optional_id::<OperationId>(
        event.operation_id.as_deref().unwrap_or(""),
        "operation_id",
    )
    .ok()
    .flatten();

    let owner_epoch = event.owner_epoch.map(OwnerEpoch)?;
    let binding_revision = event.binding_revision.map(Revision)?;
    let session_revision = event.session_revision.map(Revision)?;

    Some(MediaNodeCallback {
        media_node_id,
        media_node_instance_epoch: MediaNodeInstanceEpoch(event.media_node_instance_epoch),
        media_session_id,
        media_binding_id,
        operation_id,
        owner_epoch,
        message_id: event.event_id.clone(),
        binding_revision,
        session_revision,
        kind,
    })
}

fn map_event_payload(event: &MediaEvent) -> Result<MediaNodeCallbackKind, DomainError> {
    match event.event.as_ref() {
        Some(MediaEventPayload::StreamStarted(_))
        | Some(MediaEventPayload::RecordStarted(_))
        | Some(MediaEventPayload::StreamOnline(_))
        | Some(MediaEventPayload::RtpNegotiated(_)) => Ok(MediaNodeCallbackKind::Started),
        Some(MediaEventPayload::StreamStopped(s)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: s.reason.clone(),
        }),
        Some(MediaEventPayload::RecordStopped(_)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: "record_stopped".to_string(),
        }),
        Some(MediaEventPayload::StreamOffline(s)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: s.reason.clone(),
        }),
        Some(MediaEventPayload::PlaybackComplete(s)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: s.reason.clone(),
        }),
        Some(MediaEventPayload::RtpTimeout(_)) => Ok(MediaNodeCallbackKind::Failed {
            code: "rtp_timeout".to_string(),
            message: "RTP session timed out".to_string(),
        }),
        Some(MediaEventPayload::ProxyStateChanged(s)) => match s.state.as_str() {
            "active" | "ACTIVE" => Ok(MediaNodeCallbackKind::Started),
            "stopped" | "STOPPED" => Ok(MediaNodeCallbackKind::Stopped {
                reason: "proxy_stopped".to_string(),
            }),
            _ => Ok(MediaNodeCallbackKind::Failed {
                code: "proxy_state_error".to_string(),
                message: s.state.clone(),
            }),
        },
        Some(MediaEventPayload::ResourceStateChanged(s)) => {
            let state = s
                .new_state
                .as_ref()
                .map(|ns| ns.state.as_str())
                .unwrap_or("");
            match state {
                "active" | "ACTIVE" => Ok(MediaNodeCallbackKind::Started),
                "stopped" | "STOPPED" => Ok(MediaNodeCallbackKind::Stopped {
                    reason: "resource_stopped".to_string(),
                }),
                "error" | "ERROR" => Ok(MediaNodeCallbackKind::Failed {
                    code: "resource_state_error".to_string(),
                    message: "resource entered error state".to_string(),
                }),
                _ => Err(DomainError::invalid_argument(format!(
                    "unhandled resource state: {state}"
                ))),
            }
        }
        Some(MediaEventPayload::Error(e)) => map_media_error(e),
        Some(MediaEventPayload::SnapshotTaken(_)) => Err(DomainError::invalid_argument(
            "snapshot_taken is not a session lifecycle event",
        )),
        Some(MediaEventPayload::NodeLifecycle(_)) => Err(DomainError::invalid_argument(
            "node_lifecycle is not a session lifecycle event",
        )),
        None => Err(DomainError::invalid_argument(
            "media event payload is missing",
        )),
    }
}

#[allow(deprecated)]
fn map_media_error(
    error: &cheetah_signal_grpc::cheetah::media::v1::MediaError,
) -> Result<MediaNodeCallbackKind, DomainError> {
    if let Some(status) = &error.status {
        return Ok(MediaNodeCallbackKind::Failed {
            code: status.code.clone(),
            message: status.message.clone(),
        });
    }
    if !error.error_code.is_empty() {
        return Ok(MediaNodeCallbackKind::Failed {
            code: error.error_code.clone(),
            message: error.error_message.clone(),
        });
    }
    Err(DomainError::invalid_argument(
        "error event missing status and deprecated error_code",
    ))
}

fn parse_id_or_default<T: FromStr<Err = cheetah_signal_types::SignalError> + Default>(
    value: &str,
) -> T {
    if value.is_empty() {
        return T::default();
    }
    T::from_str(value).unwrap_or_default()
}

fn parse_optional_id<T: FromStr<Err = cheetah_signal_types::SignalError>>(
    value: &str,
    field: &str,
) -> Result<Option<T>, DomainError> {
    if value.is_empty() {
        return Ok(None);
    }
    T::from_str(value)
        .map(Some)
        .map_err(|e| DomainError::invalid_argument(format!("invalid {field}: {e}")))
}

fn parse_required_id<T: FromStr<Err = cheetah_signal_types::SignalError>>(
    value: &str,
    field: &str,
) -> Result<T, DomainError> {
    if value.is_empty() {
        return Err(DomainError::invalid_argument(format!(
            "{field} is required"
        )));
    }
    T::from_str(value).map_err(|e| DomainError::invalid_argument(format!("invalid {field}: {e}")))
}

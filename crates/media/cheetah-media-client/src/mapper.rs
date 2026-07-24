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
    OwnerEpoch, Revision, TenantId, UtcTimestamp, clamp_str,
};
use std::str::FromStr;

/// Maximum byte length of free-form media node event/command diagnostic strings.
const MAX_MEDIA_DIAGNOSTIC_BYTES: usize = 1024;
/// Maximum byte length of stable media node event/command diagnostic codes.
const MAX_MEDIA_CODE_BYTES: usize = 256;
/// Maximum byte length of media node event identifiers and correlation fields.
const MAX_MEDIA_EVENT_ID_BYTES: usize = 256;
/// Maximum byte length of W3C traceparent (55) plus headroom for invalid values.
const MAX_MEDIA_TRACEPARENT_BYTES: usize = 128;
/// Maximum byte length of W3C tracestate.
const MAX_MEDIA_TRACESTATE_BYTES: usize = 512;

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

    let error_message = result
        .error
        .as_ref()
        .map(|e| clamp_str(&e.message, MAX_MEDIA_DIAGNOSTIC_BYTES));

    match CommandStatus::try_from(result.status) {
        Ok(CommandStatus::Completed) => Ok(MediaNodeCommandResult::Completed),
        Ok(CommandStatus::Accepted) => Ok(MediaNodeCommandResult::Accepted),
        // A timeout means the media node did not confirm the outcome; the
        // side effect may or may not have been applied, so surface it as an
        // unknown outcome for the reconciler rather than a terminal failure.
        Ok(CommandStatus::Timeout) => Ok(MediaNodeCommandResult::UnknownOutcome {
            code: "timeout".to_string(),
            message: error_message.unwrap_or_else(|| "media node command timed out".to_string()),
        }),
        Ok(s) => Ok(MediaNodeCommandResult::Failed {
            code: clamp_str(&format!("{s:?}"), MAX_MEDIA_CODE_BYTES),
            message: error_message.unwrap_or_default(),
        }),
        Err(_) => Ok(MediaNodeCommandResult::Failed {
            code: "unknown_status".to_string(),
            message: error_message.unwrap_or_default(),
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
    let event_id = clamp_str(&event.event_id, MAX_MEDIA_EVENT_ID_BYTES);
    let correlation_id = clamp_str(&event.correlation_id, MAX_MEDIA_EVENT_ID_BYTES);
    let sequence = event.sequence;
    let occurred_at = event
        .occurred_at
        .as_ref()
        .and_then(UtcTimestamp::from_prost_timestamp);
    let traceparent = if event.traceparent.is_empty() {
        None
    } else {
        Some(clamp_str(&event.traceparent, MAX_MEDIA_TRACEPARENT_BYTES))
    };
    let tracestate = if event.tracestate.is_empty() {
        None
    } else {
        Some(clamp_str(&event.tracestate, MAX_MEDIA_TRACESTATE_BYTES))
    };

    let callback = try_build_callback(event, &event_id);

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

fn try_build_callback(event: &MediaEvent, clamped_event_id: &str) -> Option<MediaNodeCallback> {
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
        message_id: clamped_event_id.to_string(),
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
            reason: clamp_str(&s.reason, MAX_MEDIA_DIAGNOSTIC_BYTES),
        }),
        Some(MediaEventPayload::RecordStopped(_)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: "record_stopped".to_string(),
        }),
        Some(MediaEventPayload::StreamOffline(s)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: clamp_str(&s.reason, MAX_MEDIA_DIAGNOSTIC_BYTES),
        }),
        Some(MediaEventPayload::PlaybackComplete(s)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: clamp_str(&s.reason, MAX_MEDIA_DIAGNOSTIC_BYTES),
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
                message: clamp_str(&s.state, MAX_MEDIA_DIAGNOSTIC_BYTES),
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
                    "unhandled resource state: {}",
                    clamp_str(state, MAX_MEDIA_DIAGNOSTIC_BYTES)
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
            code: clamp_str(&status.code, MAX_MEDIA_CODE_BYTES),
            message: clamp_str(&status.message, MAX_MEDIA_DIAGNOSTIC_BYTES),
        });
    }
    if !error.error_code.is_empty() {
        return Ok(MediaNodeCallbackKind::Failed {
            code: clamp_str(&error.error_code, MAX_MEDIA_CODE_BYTES),
            message: clamp_str(&error.error_message, MAX_MEDIA_DIAGNOSTIC_BYTES),
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

#[cfg(test)]
#[allow(deprecated, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use cheetah_signal_grpc::cheetah::common::v1::{CommandResult, CommandStatus, ErrorStatus};
    use cheetah_signal_grpc::cheetah::foundation::v1::ErrorStatus as FoundationErrorStatus;
    use cheetah_signal_grpc::cheetah::media::v1::{
        MediaError, MediaEvent, MediaResourceState, MediaSessionRefStopped, PlaybackCompleteEvent,
        ProxyStateChangedEvent, ResourceStateChangedEvent, StreamOfflineEvent,
        media_event::Event as MediaEventPayload,
    };

    fn oversized_string(extra: usize) -> String {
        "x".repeat(MAX_MEDIA_DIAGNOSTIC_BYTES + extra)
    }

    #[test]
    fn map_command_result_clamps_oversized_error_message() {
        let message = oversized_string(20);
        let response = MediaControlExecuteResponse {
            result: Some(CommandResult {
                status: CommandStatus::Failed as i32,
                operation_id: "op-1".to_string(),
                error: Some(ErrorStatus {
                    code: "media_failed".to_string(),
                    message: message.clone(),
                    retryable: false,
                    violations: Vec::new(),
                }),
            }),
        };

        let result = map_command_result(response).unwrap();
        match result {
            MediaNodeCommandResult::Failed { code, message: m } => {
                // Known CommandStatus variants become the code; the wire error.code is ignored.
                assert_eq!(code, "Failed");
                assert_eq!(m.len(), MAX_MEDIA_DIAGNOSTIC_BYTES);
                assert!(m.starts_with('x'));
                assert!(!m.ends_with("x") || m.len() < message.len());
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn map_command_result_clamps_unknown_status_error_message() {
        let message = oversized_string(20);
        let response = MediaControlExecuteResponse {
            result: Some(CommandResult {
                status: 999,
                operation_id: "op-1".to_string(),
                error: Some(ErrorStatus {
                    code: "c".repeat(MAX_MEDIA_CODE_BYTES + 10),
                    message: message.clone(),
                    retryable: false,
                    violations: Vec::new(),
                }),
            }),
        };

        let result = map_command_result(response).unwrap();
        match result {
            MediaNodeCommandResult::Failed { code, message: m } => {
                // Unknown statuses map to a static code; the message is still clamped.
                assert_eq!(code, "unknown_status");
                assert_eq!(m.len(), MAX_MEDIA_DIAGNOSTIC_BYTES);
                assert!(m.starts_with('x'));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    fn sample_event_with_payload(payload: MediaEventPayload) -> MediaEvent {
        MediaEvent {
            tenant_id: "11111111-1111-1111-1111-111111111111".to_string(),
            event_id: "ev-1".to_string(),
            media_node_id: "22222222-2222-2222-2222-222222222222".to_string(),
            media_node_instance_epoch: 1,
            sequence: 1,
            occurred_at: None,
            media_session_id: "44444444-4444-4444-4444-444444444444".to_string(),
            media_binding_id: "33333333-3333-3333-3333-333333333333".to_string(),
            media_handle: String::new(),
            media_key: None,
            correlation_id: "corr-1".to_string(),
            traceparent: String::new(),
            tracestate: String::new(),
            operation_id: Some("op-1".to_string()),
            owner_epoch: Some(1),
            binding_revision: Some(0),
            session_revision: Some(0),
            event: Some(payload),
        }
    }

    #[test]
    fn map_media_event_clamps_header_fields() {
        let event = MediaEvent {
            event_id: "e".repeat(MAX_MEDIA_EVENT_ID_BYTES + 4),
            correlation_id: "c".repeat(MAX_MEDIA_EVENT_ID_BYTES + 4),
            traceparent: "t".repeat(MAX_MEDIA_TRACEPARENT_BYTES + 4),
            tracestate: "s".repeat(MAX_MEDIA_TRACESTATE_BYTES + 4),
            ..sample_event_with_payload(MediaEventPayload::RtpTimeout(
                cheetah_signal_grpc::cheetah::media::v1::RtpTimeoutEvent::default(),
            ))
        };

        let mapped = map_media_event(&event);
        assert_eq!(mapped.event_id.len(), MAX_MEDIA_EVENT_ID_BYTES);
        assert_eq!(mapped.correlation_id.len(), MAX_MEDIA_EVENT_ID_BYTES);
        assert_eq!(
            mapped.traceparent.as_ref().unwrap().len(),
            MAX_MEDIA_TRACEPARENT_BYTES
        );
        assert_eq!(
            mapped.tracestate.as_ref().unwrap().len(),
            MAX_MEDIA_TRACESTATE_BYTES
        );
    }

    #[test]
    fn map_media_event_clamps_stream_stopped_reason() {
        let reason = oversized_string(10);
        let payload = MediaEventPayload::StreamStopped(MediaSessionRefStopped {
            session: None,
            reason: reason.clone(),
        });

        let event = sample_event_with_payload(payload);
        let mapped = map_media_event(&event);
        match mapped.callback.unwrap().kind {
            MediaNodeCallbackKind::Stopped { reason: r } => {
                assert_eq!(r.len(), MAX_MEDIA_DIAGNOSTIC_BYTES);
                assert!(r.starts_with('x'));
            }
            other => panic!("expected Stopped, got {other:?}"),
        }
    }

    #[test]
    fn map_media_event_clamps_media_error_code_and_message() {
        let message = oversized_string(10);
        let code = "c".repeat(MAX_MEDIA_CODE_BYTES + 10);
        let payload = MediaEventPayload::Error(MediaError {
            session: None,
            error_code: String::new(),
            error_message: String::new(),
            status: Some(FoundationErrorStatus {
                code: code.clone(),
                message: message.clone(),
                retryable: false,
                violations: Vec::new(),
            }),
            request_id: String::new(),
            correlation_id: String::new(),
            resource_ref: None,
            outcome: 0,
        });

        let event = sample_event_with_payload(payload);
        let mapped = map_media_event(&event);
        match mapped.callback.unwrap().kind {
            MediaNodeCallbackKind::Failed {
                code: c,
                message: m,
            } => {
                assert_eq!(c.len(), MAX_MEDIA_CODE_BYTES);
                assert_eq!(m.len(), MAX_MEDIA_DIAGNOSTIC_BYTES);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn map_media_event_clamps_proxy_state_string() {
        let huge = oversized_string(10);
        let payload = MediaEventPayload::ProxyStateChanged(ProxyStateChangedEvent { state: huge });

        let event = sample_event_with_payload(payload);
        let mapped = map_media_event(&event);
        match mapped.callback.unwrap().kind {
            MediaNodeCallbackKind::Failed { message, .. } => {
                assert_eq!(message.len(), MAX_MEDIA_DIAGNOSTIC_BYTES);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn map_media_event_does_not_build_callback_for_oversized_unhandled_resource_state() {
        let huge = oversized_string(10);
        let payload = MediaEventPayload::ResourceStateChanged(ResourceStateChangedEvent {
            new_state: Some(MediaResourceState {
                kind: String::new(),
                state: huge,
                generation: 0,
                ..Default::default()
            }),
            old_state: None,
        });

        let event = sample_event_with_payload(payload);
        let mapped = map_media_event(&event);
        assert!(mapped.callback.is_none());
    }

    #[test]
    fn map_media_event_clamps_playback_complete_and_stream_offline_reason() {
        let reason = oversized_string(10);
        let playback = MediaEventPayload::PlaybackComplete(PlaybackCompleteEvent {
            media_session_id: String::new(),
            media_handle: String::new(),
            reason: reason.clone(),
        });
        let offline = MediaEventPayload::StreamOffline(StreamOfflineEvent {
            media_session_id: String::new(),
            media_handle: String::new(),
            reason,
        });

        for payload in [playback, offline] {
            let event = sample_event_with_payload(payload);
            let mapped = map_media_event(&event);
            match mapped.callback.unwrap().kind {
                MediaNodeCallbackKind::Stopped { reason: r } => {
                    assert_eq!(r.len(), MAX_MEDIA_DIAGNOSTIC_BYTES);
                }
                other => panic!("expected Stopped, got {other:?}"),
            }
        }
    }
}

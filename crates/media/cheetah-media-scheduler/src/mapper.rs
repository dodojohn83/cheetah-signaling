//! Explicit domain-to-proto and proto-to-domain mapping for the media port.

use cheetah_domain::{
    DomainError, MediaNodeCallback, MediaNodeCallbackKind, MediaNodeCommand, MediaNodeSessionRef,
};
use cheetah_signal_contracts::cheetah::media::v1::{
    MediaCommand, MediaControlPayload, MediaError, MediaEvent, MediaSessionRef, media_command,
    media_event,
};
use cheetah_signal_types::{MediaNodeInstanceEpoch, OperationId, OwnerEpoch, Revision, TenantId};
use std::str::FromStr;

/// Maps a typed domain media command to a typed proto `MediaCommand`.
///
/// `StopMediaSession` is mapped to the explicit `stop_rtp` variant; control and
/// playback commands currently map to the `control` variant carrying a JSON
/// encoding of the typed `CommandPayload`. This preserves the existing contract
/// while making the conversion explicit and giving field-level errors.
pub fn map_command_to_media_command(
    command: &MediaNodeCommand,
) -> Result<MediaCommand, DomainError> {
    let target_media_node_instance_epoch = command.media_node_instance_epoch.0;

    let command = match &command.payload {
        cheetah_domain::CommandPayload::StopMediaSession { media_session_id } => {
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

/// Maps a proto `MediaSessionRef` to the domain `MediaNodeSessionRef`.
///
/// Empty optional fields are treated as `None`; non-empty malformed fields
/// surface as `InvalidArgument` errors with the offending field name so that
/// malformed list responses are reported as contract violations.
pub fn map_proto_session_ref(
    tenant_id: cheetah_signal_types::TenantId,
    media_node_id: cheetah_signal_types::NodeId,
    proto: &MediaSessionRef,
) -> Result<MediaNodeSessionRef, DomainError> {
    let _ = tenant_id;
    let _ = media_node_id;

    let media_session_id = parse_optional_id(&proto.media_session_id, "media_session_id")?
        .ok_or_else(|| DomainError::invalid_argument("media_session_id is required"))?;
    let device_id = parse_optional_id(&proto.device_id, "device_id")?;
    let channel_id = parse_optional_id(&proto.channel_id, "channel_id")?;

    Ok(MediaNodeSessionRef {
        media_session_id,
        device_id,
        channel_id,
        media_node_instance_epoch: cheetah_signal_types::MediaNodeInstanceEpoch(
            proto.media_node_instance_epoch,
        ),
    })
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

/// Maps a proto `MediaEvent` to a domain `(tenant_id, MediaNodeCallback)` pair.
///
/// Unknown or non-session lifecycle events are rejected with a field-level error
/// so the consumer can log a diagnostic and skip them without treating them as
/// transient failures.
pub fn map_media_event_to_callback(
    event: &MediaEvent,
) -> Result<(TenantId, MediaNodeCallback), DomainError> {
    if event.event_id.is_empty() {
        return Err(DomainError::invalid_argument("event_id is required"));
    }
    let tenant_id = parse_required_id(&event.tenant_id, "tenant_id")?;
    let media_node_id = parse_required_id(&event.media_node_id, "media_node_id")?;
    let media_session_id = parse_required_id(&event.media_session_id, "media_session_id")?;
    let media_binding_id = parse_required_id(&event.media_binding_id, "media_binding_id")?;
    let operation_id = parse_optional_id::<OperationId>(
        event.operation_id.as_deref().unwrap_or(""),
        "operation_id",
    )?;

    let kind = map_event_payload(event)?;

    // owner_epoch, binding_revision and session_revision are required for
    // lifecycle callbacks so that stale/late events can be fenced. Older media
    // nodes that do not populate these fields should not emit session lifecycle
    // events; if they do, the event is rejected as a diagnostic.
    let owner_epoch = event
        .owner_epoch
        .map(OwnerEpoch)
        .ok_or_else(|| DomainError::invalid_argument("owner_epoch is required"))?;
    let binding_revision = event
        .binding_revision
        .map(Revision)
        .ok_or_else(|| DomainError::invalid_argument("binding_revision is required"))?;
    let session_revision = event
        .session_revision
        .map(Revision)
        .ok_or_else(|| DomainError::invalid_argument("session_revision is required"))?;

    let callback = MediaNodeCallback {
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
    };

    Ok((tenant_id, callback))
}

fn map_event_payload(event: &MediaEvent) -> Result<MediaNodeCallbackKind, DomainError> {
    match event.event.as_ref() {
        Some(media_event::Event::StreamStarted(_))
        | Some(media_event::Event::RecordStarted(_))
        | Some(media_event::Event::StreamOnline(_))
        | Some(media_event::Event::RtpNegotiated(_)) => Ok(MediaNodeCallbackKind::Started),
        Some(media_event::Event::StreamStopped(s)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: s.reason.clone(),
        }),
        Some(media_event::Event::RecordStopped(_)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: "record_stopped".to_string(),
        }),
        Some(media_event::Event::StreamOffline(s)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: s.reason.clone(),
        }),
        Some(media_event::Event::PlaybackComplete(s)) => Ok(MediaNodeCallbackKind::Stopped {
            reason: s.reason.clone(),
        }),
        Some(media_event::Event::RtpTimeout(_)) => Ok(MediaNodeCallbackKind::Failed {
            code: "rtp_timeout".to_string(),
            message: "RTP session timed out".to_string(),
        }),
        Some(media_event::Event::ProxyStateChanged(s)) => match s.state.as_str() {
            "active" | "ACTIVE" => Ok(MediaNodeCallbackKind::Started),
            "stopped" | "STOPPED" => Ok(MediaNodeCallbackKind::Stopped {
                reason: "proxy_stopped".to_string(),
            }),
            _ => Ok(MediaNodeCallbackKind::Failed {
                code: "proxy_state_error".to_string(),
                message: s.state.clone(),
            }),
        },
        Some(media_event::Event::ResourceStateChanged(s)) => {
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
        Some(media_event::Event::Error(e)) => map_media_error(e),
        Some(media_event::Event::SnapshotTaken(_)) => Err(DomainError::invalid_argument(
            "snapshot_taken is not a session lifecycle event",
        )),
        Some(media_event::Event::NodeLifecycle(_)) => Err(DomainError::invalid_argument(
            "node_lifecycle is not a session lifecycle event",
        )),
        None => Err(DomainError::invalid_argument(
            "media event payload is missing",
        )),
    }
}

#[allow(deprecated)]
fn map_media_error(error: &MediaError) -> Result<MediaNodeCallbackKind, DomainError> {
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use cheetah_domain::CommandPayload;
    use cheetah_signal_types::{
        MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId, OperationId, OwnerEpoch,
        TenantId,
    };
    use std::str::FromStr;

    fn sample_command(payload: CommandPayload) -> MediaNodeCommand {
        MediaNodeCommand {
            request_id: "req-1".to_string(),
            tenant_id: TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap(),
            media_session_id: MediaSessionId::from_str("22222222-2222-2222-2222-222222222222")
                .unwrap(),
            media_binding_id: MediaBindingId::from_str("33333333-3333-3333-3333-333333333333")
                .unwrap(),
            media_node_id: NodeId::from_str("44444444-4444-4444-4444-444444444444").unwrap(),
            media_node_instance_epoch: MediaNodeInstanceEpoch(7),
            operation_id: OperationId::from_str("55555555-5555-5555-5555-555555555555").unwrap(),
            owner_epoch: OwnerEpoch(1),
            source_node_id: NodeId::from_str("66666666-6666-6666-6666-666666666666").unwrap(),
            deadline: None,
            idempotency_key: "idem-1".to_string(),
            contract_version: 1,
            payload,
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn stop_media_session_maps_to_stop_rtp() {
        let session = MediaSessionId::from_str("22222222-2222-2222-2222-222222222222").unwrap();
        let command = sample_command(CommandPayload::StopMediaSession {
            media_session_id: session,
        });
        let proto = map_command_to_media_command(&command).unwrap();
        assert_eq!(proto.target_media_node_instance_epoch, 7);
        match proto.command.unwrap() {
            media_command::Command::StopRtp(r) => {
                assert_eq!(r.media_session_id, session.to_string());
            }
            other => panic!("expected StopRtp, got {other:?}"),
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn control_playback_maps_to_control_payload() {
        let session = MediaSessionId::from_str("22222222-2222-2222-2222-222222222222").unwrap();
        let command = sample_command(CommandPayload::ControlPlayback {
            media_session_id: session,
            command: cheetah_domain::MediaControl::Pause,
        });
        let proto = map_command_to_media_command(&command).unwrap();
        assert_eq!(proto.target_media_node_instance_epoch, 7);
        match proto.command.unwrap() {
            media_command::Command::Control(c) => {
                assert_eq!(c.command_type, "ControlPlayback");
                assert_eq!(c.media_session_id, session.to_string());
                assert!(!c.payload.is_empty());
            }
            other => panic!("expected Control, got {other:?}"),
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn map_session_ref_parses_valid_optional_ids() {
        let proto = MediaSessionRef {
            media_session_id: "22222222-2222-2222-2222-222222222222".to_string(),
            device_id: "11111111-1111-1111-1111-111111111111".to_string(),
            channel_id: "33333333-3333-3333-3333-333333333333".to_string(),
            media_node_instance_epoch: 7,
        };
        let result = map_proto_session_ref(
            TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap(),
            NodeId::from_str("44444444-4444-4444-4444-444444444444").unwrap(),
            &proto,
        )
        .unwrap();
        assert_eq!(
            result.media_session_id.to_string(),
            "22222222-2222-2222-2222-222222222222"
        );
        assert!(result.device_id.is_some());
        assert!(result.channel_id.is_some());
        assert_eq!(result.media_node_instance_epoch.0, 7);
    }

    #[test]
    fn map_session_ref_rejects_empty_session_id() {
        let proto = MediaSessionRef {
            media_session_id: String::new(),
            device_id: String::new(),
            channel_id: String::new(),
            media_node_instance_epoch: 7,
        };
        let result = map_proto_session_ref(
            TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap(),
            NodeId::from_str("44444444-4444-4444-4444-444444444444").unwrap(),
            &proto,
        );
        assert!(result.is_err());
    }

    #[test]
    fn map_session_ref_rejects_invalid_session_id() {
        let proto = MediaSessionRef {
            media_session_id: "not-a-uuid".to_string(),
            device_id: String::new(),
            channel_id: String::new(),
            media_node_instance_epoch: 7,
        };
        let result = map_proto_session_ref(
            TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap(),
            NodeId::from_str("44444444-4444-4444-4444-444444444444").unwrap(),
            &proto,
        );
        assert!(result.is_err());
    }
}

//! Outbound command handling for GB28181 access.

use super::Gb28181Access;
use crate::command::Gb28181Command;
use crate::events::Gb28181Event;
use crate::ports::CredentialProvider;
use crate::xml::{
    DeviceControlKind, DeviceControlRequest, PresetAction as GbPresetAction, PresetCommand,
    PtzCommand, QueryRequest,
};
use cheetah_domain::{CommandPayload, PtzDirection, QueryKind};
use cheetah_gb28181_core::{
    AccessOutput, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
};

pub(crate) fn process_command<P: CredentialProvider>(
    access: &mut Gb28181Access<P>,
    input: Gb28181Command,
) -> Result<Vec<AccessOutput<Gb28181Event>>, crate::error::AccessError> {
    let target = access
        .device_send_target(&input.device_external_id)
        .ok_or(crate::error::AccessError::NotRegistered)?;

    let sn = next_sn(access);
    let tag = access.next_tag();
    let target_id = input
        .channel_external_id
        .as_ref()
        .unwrap_or(&input.device_external_id);

    let body = match input.command.payload() {
        CommandPayload::Query { query } => {
            QueryRequest::from_command(&sn, target_id.as_ref(), query).encode_xml()
        }
        CommandPayload::Ptz {
            channel_id: _,
            direction,
            speed,
        } => DeviceControlRequest {
            sn,
            device_id: target_id.as_ref().to_string(),
            kind: DeviceControlKind::Ptz(ptz_command(direction.clone(), *speed)),
        }
        .encode_xml(),
        CommandPayload::Preset { preset } => {
            if preset.action == cheetah_domain::PresetAction::List {
                QueryRequest {
                    sn,
                    device_id: target_id.as_ref().to_string(),
                    kind: QueryKind::PresetQuery,
                    start_time: None,
                    end_time: None,
                    config_type: None,
                    scale: None,
                }
                .encode_xml()
            } else {
                let action = match preset.action {
                    cheetah_domain::PresetAction::Goto => GbPresetAction::Call,
                    cheetah_domain::PresetAction::Set => GbPresetAction::Set,
                    cheetah_domain::PresetAction::Delete => GbPresetAction::Delete,
                    cheetah_domain::PresetAction::List => unreachable!(),
                    _ => {
                        return Err(crate::error::AccessError::UnsupportedCmdType(
                            "preset".to_string(),
                        ));
                    }
                };
                let point = u8::try_from(preset.preset_id).map_err(|_| {
                    crate::error::AccessError::InvalidXml("preset_id exceeds u8 range".to_string())
                })?;
                DeviceControlRequest {
                    sn,
                    device_id: target_id.as_ref().to_string(),
                    kind: DeviceControlKind::Preset(PresetCommand { action, point }),
                }
                .encode_xml()
            }
        }
        CommandPayload::DeviceControl { control } => {
            let kind = match control.kind {
                cheetah_domain::DeviceControlKind::Guard => {
                    DeviceControlKind::Guard(control.enabled.unwrap_or(true))
                }
                cheetah_domain::DeviceControlKind::AlarmReset => DeviceControlKind::AlarmReset {
                    alarm_method: control.param.clone(),
                },
                cheetah_domain::DeviceControlKind::Record => {
                    DeviceControlKind::Record(control.enabled.unwrap_or(true))
                }
                cheetah_domain::DeviceControlKind::TeleBoot => DeviceControlKind::TeleBoot,
                cheetah_domain::DeviceControlKind::IFrame => DeviceControlKind::IFrame,
                cheetah_domain::DeviceControlKind::DeviceConfig => {
                    DeviceControlKind::DeviceConfig {
                        config_type: control.param.clone(),
                    }
                }
            };
            DeviceControlRequest {
                sn,
                device_id: target_id.as_ref().to_string(),
                kind,
            }
            .encode_xml()
        }
        other => {
            return Err(crate::error::AccessError::UnsupportedCmdType(
                other.kind().to_string(),
            ));
        }
    }?;

    let message = build_message_request(access, target_id.as_ref(), body.into_bytes(), &tag)?;
    Ok(vec![AccessOutput::SendMessage { target, message }])
}

fn next_sn<P: CredentialProvider>(access: &Gb28181Access<P>) -> String {
    let n = access
        .tag_counter
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    n.to_string()
}

fn build_message_request<P: CredentialProvider>(
    access: &Gb28181Access<P>,
    target_id: &str,
    body: Vec<u8>,
    tag: &str,
) -> Result<SipMessage, crate::error::AccessError> {
    let from_uri = SipUri::parse(format!(
        "sip:{}@{}",
        access.config.domain_id().as_ref(),
        access.config.realm()
    ))
    .map_err(|e| crate::error::AccessError::Internal(e.to_string()))?;
    let to_uri = SipUri::parse(format!("sip:{target_id}@{}", access.config.realm()))
        .map_err(|e| crate::error::AccessError::Internal(e.to_string()))?;
    let branch = format!("z9hG4bK{tag}");
    let call_id = format!("gb-cmd-{tag}@{}", access.config.realm());

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", access.config.realm(), 5060, &branch)
            .map_err(|e| crate::error::AccessError::Internal(e.to_string()))?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&from_uri, tag)
            .map_err(|e| crate::error::AccessError::Internal(e.to_string()))?,
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&to_uri));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(1, Method::Message));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(
        HeaderName::UserAgent,
        HeaderValue::new("Cheetah-GB28181/0.1.0"),
    );
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Message, to_uri),
        headers,
        body,
    })
}

fn ptz_command(direction: PtzDirection, speed: f64) -> PtzCommand {
    let mut cmd = PtzCommand::new();
    let scale = speed.clamp(0.0, 1.0);
    let pan_tilt_speed = (scale * cmd.max_pan_tilt_speed as f64).round() as i8;
    let zoom_speed = (scale * cmd.max_zoom_speed as f64).round() as i8;

    match direction {
        PtzDirection::Stop => {}
        PtzDirection::Up => cmd.tilt = pan_tilt_speed,
        PtzDirection::Down => cmd.tilt = -pan_tilt_speed,
        PtzDirection::Left => cmd.pan = -pan_tilt_speed,
        PtzDirection::Right => cmd.pan = pan_tilt_speed,
        PtzDirection::UpLeft => {
            cmd.pan = -pan_tilt_speed;
            cmd.tilt = pan_tilt_speed;
        }
        PtzDirection::UpRight => {
            cmd.pan = pan_tilt_speed;
            cmd.tilt = pan_tilt_speed;
        }
        PtzDirection::DownLeft => {
            cmd.pan = -pan_tilt_speed;
            cmd.tilt = -pan_tilt_speed;
        }
        PtzDirection::DownRight => {
            cmd.pan = pan_tilt_speed;
            cmd.tilt = -pan_tilt_speed;
        }
        PtzDirection::ZoomIn => cmd.zoom = zoom_speed,
        PtzDirection::ZoomOut => cmd.zoom = -zoom_speed,
        _ => {
            return cmd;
        }
    }
    cmd
}

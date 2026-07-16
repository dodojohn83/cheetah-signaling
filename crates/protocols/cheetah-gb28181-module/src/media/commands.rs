//! Command dispatch for GB28181 media sessions.

use super::control::{PlaybackAction, build_info_mansrtsp};
use super::invite::{SdpParams, build_bye, build_cancel, build_invite};
use super::session::{Session, SessionState};
use super::{Gb28181Media, MediaCommand, MediaError, MediaOutput};
use cheetah_gb28181_core::sdp::{SdpAttribute, SdpDirection, SdpTime};

/// Shared fields for every start-media command.
struct StartParams {
    media_session_id: super::MediaSessionId,
    channel_id: super::ChannelId,
    device_id: super::DeviceId,
    target: cheetah_gb28181_core::SipUri,
    call_id: String,
    local_tag: String,
    cseq: u32,
    branch: String,
    subject_session: String,
    media_address: String,
    media_port: u16,
}

/// Dispatches a high-level command to SIP/SDP outputs.
pub(super) fn on_command(
    media: &mut Gb28181Media,
    cmd: MediaCommand,
) -> Result<Vec<MediaOutput>, MediaError> {
    match cmd {
        MediaCommand::StartLive {
            media_session_id,
            channel_id,
            device_id,
            target,
            call_id,
            local_tag,
            cseq,
            branch,
            subject_session,
            media_address,
            media_port,
            ssrc,
            transport,
        } => {
            let p = StartParams {
                media_session_id,
                channel_id,
                device_id,
                target,
                call_id,
                local_tag,
                cseq,
                branch,
                subject_session,
                media_address,
                media_port,
            };
            let sdp = SdpParams {
                session_name: "Play".to_string(),
                media_type: "video".to_string(),
                media_port: p.media_port,
                transport,
                direction: SdpDirection::RecvOnly,
                time: SdpTime {
                    start: "0".to_string(),
                    stop: "0".to_string(),
                },
                ssrc: Some(ssrc),
                media_address: p.media_address.clone(),
                rtpmap: Some(SdpParams::default_video_rtpmap()),
                extra_attrs: Vec::new(),
            };
            do_start(media, &p, &sdp)
        }
        MediaCommand::StartPlayback {
            media_session_id,
            channel_id,
            device_id,
            target,
            call_id,
            local_tag,
            cseq,
            branch,
            subject_session,
            media_address,
            media_port,
            ssrc,
            transport,
            start_time,
            end_time,
        } => {
            let p = StartParams {
                media_session_id,
                channel_id,
                device_id,
                target,
                call_id,
                local_tag,
                cseq,
                branch,
                subject_session,
                media_address,
                media_port,
            };
            let sdp = SdpParams {
                session_name: "Playback".to_string(),
                media_type: "video".to_string(),
                media_port: p.media_port,
                transport,
                direction: SdpDirection::RecvOnly,
                time: SdpTime {
                    start: start_time,
                    stop: end_time,
                },
                ssrc: Some(ssrc),
                media_address: p.media_address.clone(),
                rtpmap: Some(SdpParams::default_video_rtpmap()),
                extra_attrs: Vec::new(),
            };
            do_start(media, &p, &sdp)
        }
        MediaCommand::StartDownload {
            media_session_id,
            channel_id,
            device_id,
            target,
            call_id,
            local_tag,
            cseq,
            branch,
            subject_session,
            media_address,
            media_port,
            ssrc,
            transport,
            start_time,
            end_time,
            download_speed,
        } => {
            let p = StartParams {
                media_session_id,
                channel_id,
                device_id,
                target,
                call_id,
                local_tag,
                cseq,
                branch,
                subject_session,
                media_address,
                media_port,
            };
            let extra = vec![SdpAttribute::Unknown {
                name: "downloadspeed".to_string(),
                value: Some(download_speed.to_string()),
            }];
            let sdp = SdpParams {
                session_name: "Download".to_string(),
                media_type: "video".to_string(),
                media_port: p.media_port,
                transport,
                direction: SdpDirection::RecvOnly,
                time: SdpTime {
                    start: start_time,
                    stop: end_time,
                },
                ssrc: Some(ssrc),
                media_address: p.media_address.clone(),
                rtpmap: Some(SdpParams::default_video_rtpmap()),
                extra_attrs: extra,
            };
            do_start(media, &p, &sdp)
        }
        MediaCommand::StartTalk {
            media_session_id,
            channel_id,
            device_id,
            target,
            call_id,
            local_tag,
            cseq,
            branch,
            subject_session,
            media_address,
            media_port,
            codec,
            transport,
        } => {
            let rtpmap = match codec.as_str() {
                "G.711A" => SdpParams::pcma_rtpmap(),
                "PCMA" => SdpParams::pcma_rtpmap(),
                "G.711U" => SdpParams::pcmu_rtpmap(),
                "PCMU" => SdpParams::pcmu_rtpmap(),
                _ => return Err(MediaError::Unsupported(codec)),
            };
            let p = StartParams {
                media_session_id,
                channel_id,
                device_id,
                target,
                call_id,
                local_tag,
                cseq,
                branch,
                subject_session,
                media_address,
                media_port,
            };
            let sdp = SdpParams {
                session_name: "Talk".to_string(),
                media_type: "audio".to_string(),
                media_port: p.media_port,
                transport,
                direction: SdpDirection::SendRecv,
                time: SdpTime {
                    start: "0".to_string(),
                    stop: "0".to_string(),
                },
                ssrc: None,
                media_address: p.media_address.clone(),
                rtpmap: Some(rtpmap),
                extra_attrs: Vec::new(),
            };
            do_start(media, &p, &sdp)
        }
        MediaCommand::ControlPlayback {
            media_session_id,
            action,
            scale,
            range,
        } => do_control_playback(media, media_session_id, action, scale, range),
        MediaCommand::StopMediaSession { media_session_id } => do_stop(media, media_session_id),
    }
}

fn do_start(
    media: &mut Gb28181Media,
    p: &StartParams,
    sdp: &SdpParams,
) -> Result<Vec<MediaOutput>, MediaError> {
    if media.sessions.len() >= media.config.max_sessions {
        return Err(MediaError::SessionTableFull);
    }
    if media.sessions.contains_key(&p.media_session_id) {
        return Err(MediaError::AlreadyExists);
    }

    let invite = build_invite(
        &media.config.local_sip_uri,
        &p.target,
        &p.call_id,
        &p.local_tag,
        p.cseq,
        &p.branch,
        &p.device_id,
        &p.subject_session,
        sdp,
    )
    .map_err(|e| MediaError::MalformedSip(e.to_string()))?;

    media.sessions.insert(
        p.media_session_id,
        Session {
            media_session_id: p.media_session_id,
            channel_id: p.channel_id,
            device_id: p.device_id.clone(),
            call_id: p.call_id.clone(),
            local_tag: p.local_tag.clone(),
            remote_tag: None,
            cseq: p.cseq,
            branch: p.branch.clone(),
            target: p.target.clone(),
            remote_target: None,
            state: SessionState::Inviting,
            media_address: p.media_address.clone(),
            media_port: p.media_port,
        },
    );
    media
        .call_index
        .insert(p.call_id.clone(), p.media_session_id);

    Ok(vec![MediaOutput::SendMessage(invite)])
}

fn do_stop(
    media: &mut Gb28181Media,
    media_session_id: super::MediaSessionId,
) -> Result<Vec<MediaOutput>, MediaError> {
    let session = media
        .sessions
        .get_mut(&media_session_id)
        .ok_or(MediaError::SessionNotFound)?;
    if session.state == SessionState::Stopping || session.state == SessionState::Terminated {
        return Err(MediaError::InvalidState(format!("{:?}", session.state)));
    }

    // A pending INVITE must be cancelled rather than torn down with BYE.
    if session.state == SessionState::Inviting {
        let cancel = build_cancel(
            &media.config.local_sip_uri,
            session,
            session.cseq,
            &session.branch,
            &session.target,
        );
        session.state = SessionState::Stopping;
        return Ok(vec![MediaOutput::SendMessage(cancel)]);
    }

    let next_cseq = session.cseq + 1;
    let branch = format!("{}-bye", session.branch);
    let target = session.remote_target.as_ref().unwrap_or(&session.target);
    let bye = build_bye(
        &media.config.local_sip_uri,
        session,
        next_cseq,
        &branch,
        target,
    )
    .map_err(|e| MediaError::MalformedSip(e.to_string()))?;
    session.cseq = next_cseq;
    session.state = SessionState::Stopping;

    Ok(vec![MediaOutput::SendMessage(bye)])
}

fn do_control_playback(
    media: &mut Gb28181Media,
    media_session_id: super::MediaSessionId,
    action: PlaybackAction,
    scale: Option<f64>,
    range: Option<String>,
) -> Result<Vec<MediaOutput>, MediaError> {
    let session = media
        .sessions
        .get_mut(&media_session_id)
        .ok_or(MediaError::SessionNotFound)?;
    if session.state != SessionState::Active {
        return Err(MediaError::InvalidState(format!("{:?}", session.state)));
    }

    let next_cseq = session.cseq + 1;
    let branch = format!(
        "{}-{}-info-{next_cseq}",
        session.branch,
        action.method().to_lowercase()
    );
    let target = session.remote_target.as_ref().unwrap_or(&session.target);
    let info = build_info_mansrtsp(
        &media.config.local_sip_uri,
        session,
        next_cseq,
        &branch,
        target,
        action,
        scale,
        range.as_deref(),
    )
    .map_err(|e| MediaError::MalformedSip(e.to_string()))?;
    session.cseq = next_cseq;

    Ok(vec![MediaOutput::SendMessage(info)])
}

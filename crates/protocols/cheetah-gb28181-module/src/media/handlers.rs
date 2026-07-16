//! SIP response and request handlers for GB28181 media sessions.

use super::invite::{build_ack, build_ok_response, first_contact_uri, tag_from_header};
use super::session::{SessionState, failed_event, socket_addr, stopped_event};
use super::{Gb28181Media, MediaError, MediaOutput};
use crate::events::Gb28181Event;
use cheetah_gb28181_core::{HeaderName, Method, SipMessage};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub(super) fn on_message(
    media: &mut Gb28181Media,
    msg: SipMessage,
) -> Result<Vec<MediaOutput>, MediaError> {
    let call_id = msg
        .call_id()
        .ok_or_else(|| MediaError::MalformedSip("missing Call-ID".to_string()))?
        .to_string();
    let sid = *media
        .call_index
        .get(&call_id)
        .ok_or(MediaError::SessionNotFound)?;

    match &msg {
        SipMessage::Response { line, .. } => on_response(media, sid, line.code, msg.clone()),
        SipMessage::Request { line, .. } => on_request(media, sid, &line.method, msg.clone()),
    }
}

fn on_response(
    media: &mut Gb28181Media,
    sid: super::MediaSessionId,
    code: u16,
    msg: SipMessage,
) -> Result<Vec<MediaOutput>, MediaError> {
    let cseq = msg
        .cseq()
        .ok_or_else(|| MediaError::MalformedSip("missing or malformed CSeq".to_string()))?;

    if cseq.1 == Method::Invite {
        let cseq_match = media
            .sessions
            .get(&sid)
            .map(|s| s.cseq == cseq.0)
            .unwrap_or(false);
        if cseq_match {
            if (200..300).contains(&code) {
                return on_invite_success(media, sid, msg);
            }
            if code >= 300 {
                let session = media
                    .remove_session(sid)
                    .ok_or(MediaError::SessionNotFound)?;
                let event = failed_event(&session, &media.config.domain_id, "invite rejected");
                return Ok(vec![MediaOutput::EmitEvent(event)]);
            }
            // 1xx provisional: no action yet.
            return Ok(Vec::new());
        }
    }

    if cseq.1 == Method::Bye {
        let session = media
            .remove_session(sid)
            .ok_or(MediaError::SessionNotFound)?;
        let event = stopped_event(&session, &media.config.domain_id);
        return Ok(vec![MediaOutput::EmitEvent(event)]);
    }

    // INFO responses for playback control are not tracked at this layer.
    Ok(Vec::new())
}

fn on_request(
    media: &mut Gb28181Media,
    sid: super::MediaSessionId,
    method: &Method,
    msg: SipMessage,
) -> Result<Vec<MediaOutput>, MediaError> {
    if method == &Method::Bye {
        let session = media
            .remove_session(sid)
            .ok_or(MediaError::SessionNotFound)?;
        let ok = build_ok_response(&msg);
        let event = stopped_event(&session, &media.config.domain_id);
        return Ok(vec![
            MediaOutput::SendMessage(ok),
            MediaOutput::EmitEvent(event),
        ]);
    }
    // CANCEL for an outstanding INVITE is handled by the transaction layer.
    Ok(Vec::new())
}

fn on_invite_success(
    media: &mut Gb28181Media,
    sid: super::MediaSessionId,
    msg: SipMessage,
) -> Result<Vec<MediaOutput>, MediaError> {
    let session = media
        .sessions
        .get_mut(&sid)
        .ok_or(MediaError::SessionNotFound)?;

    // If we already asked to stop/cancel the pending INVITE, do not establish the dialog.
    if session.state != SessionState::Inviting {
        let session = media
            .remove_session(sid)
            .ok_or(MediaError::SessionNotFound)?;
        let event = failed_event(&session, &media.config.domain_id, "late 200 OK after stop");
        return Ok(vec![MediaOutput::EmitEvent(event)]);
    }

    let remote_tag = tag_from_header(&msg, &HeaderName::To)
        .ok_or_else(|| MediaError::MalformedSip("missing To tag in 200 OK".to_string()))?;
    let contact = first_contact_uri(&msg)?;
    let parsed_remote_sdp = cheetah_gb28181_core::parse_sdp(msg.body(), &Default::default())
        .map_err(|e| MediaError::MalformedSdp(e.to_string()))?;
    let remote_sdp_text = String::from_utf8_lossy(msg.body()).to_string();

    let remote_ssrc = parsed_remote_sdp
        .media
        .first()
        .and_then(|m| m.y_ssrc().map(|s| s.to_string()));
    let remote_proto = parsed_remote_sdp
        .media
        .first()
        .map(|m| m.proto.clone())
        .unwrap_or_default();
    let remote_port = parsed_remote_sdp.media.first().map(|m| m.port).unwrap_or(0);
    let remote_address = parsed_remote_sdp
        .connection
        .as_ref()
        .or_else(|| {
            parsed_remote_sdp
                .media
                .first()
                .and_then(|m| m.connection.as_ref())
        })
        .map(|c| c.address.clone())
        .unwrap_or_default();

    session.remote_tag = Some(remote_tag.clone());
    session.remote_target = Some(contact.clone());

    let ack_branch = format!("{}-ack", session.branch);
    let ack = build_ack(
        &media.config.local_sip_uri,
        session,
        &remote_tag,
        &contact,
        &ack_branch,
    );

    session.state = SessionState::Active;

    let source = socket_addr(&remote_address, remote_port)
        .unwrap_or(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));

    let event = Gb28181Event::MediaSessionStarted {
        domain_id: media.config.domain_id.clone(),
        media_session_id: session.media_session_id,
        channel_id: session.channel_id,
        device_id: session.device_id.clone(),
        source,
        remote_sdp: remote_sdp_text,
        remote_ssrc,
        remote_port,
        remote_proto,
    };

    Ok(vec![
        MediaOutput::SendMessage(ack),
        MediaOutput::EmitEvent(event),
    ])
}

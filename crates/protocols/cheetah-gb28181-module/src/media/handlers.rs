//! SIP response and request handlers for GB28181 media sessions.

use super::invite::{build_ack, build_bye, build_ok_response, first_contact_uri, tag_from_header};
use super::session::{SessionState, failed_event, socket_addr, stopped_event};
use super::{Gb28181Media, MediaError, MediaOutput};
use crate::events::Gb28181Event;
use cheetah_gb28181_core::sdp::{SdpAttribute, SdpSession};
use cheetah_gb28181_core::{
    CompatibilityProfile, HeaderName, Method, SdpParserConfig, SipMessage, SipUri,
};

/// Conservative SDP parser limits for bodies received from remote devices.
const REMOTE_SDP_CONFIG: SdpParserConfig = SdpParserConfig {
    max_lines: 256,
    max_line_len: 1024,
    max_size: 16 * 1024,
    max_media: 4,
    max_attributes: 64,
    max_unknown_attributes: 32,
};

pub(super) fn on_message(
    media: &mut Gb28181Media,
    msg: SipMessage,
) -> Result<Vec<MediaOutput>, MediaError> {
    let call_id = msg
        .call_id()
        .ok_or_else(|| MediaError::MalformedSip("missing Call-ID".to_string()))?
        .to_string();
    let Some(&sid) = media.call_index.get(&call_id) else {
        // Final responses to BYE or CANCEL may be retransmissions after the dialog
        // was already torn down.
        if msg
            .cseq()
            .is_ok_and(|(_, method)| method == Method::Bye || method == Method::Cancel)
        {
            return Ok(Vec::new());
        }
        return Err(MediaError::SessionNotFound);
    };

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
        .map_err(|e| MediaError::MalformedSip(e.to_string()))?;

    if cseq.1 == Method::Invite {
        let cseq_match = media
            .sessions
            .get(&sid)
            .map(|s| s.invite_cseq == cseq.0)
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

    if cseq.1 == Method::Bye && code >= 200 {
        if let Some(session) = media.remove_session(sid) {
            let event = stopped_event(&session, &media.config.domain_id);
            return Ok(vec![MediaOutput::EmitEvent(event)]);
        }
        // Final BYE response retransmission after the session was already torn down.
        return Ok(Vec::new());
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
    let state = media
        .sessions
        .get(&sid)
        .map(|s| s.state)
        .ok_or(MediaError::SessionNotFound)?;

    let remote_tag = tag_from_header(&msg, &HeaderName::To);
    let contact = first_contact_uri(&msg);

    if state == SessionState::Active {
        // Retransmitted 200 OK for an already-active session: just re-ACK.
        // A retransmitted 200 OK may omit the tag or be malformed; fall back to the
        // tag/Contact we already recorded when the dialog became active.
        let session = media
            .sessions
            .get(&sid)
            .ok_or(MediaError::SessionNotFound)?;
        let remote_tag = remote_tag.as_deref().or(session.remote_tag.as_deref());
        let target = session.remote_target.as_ref().unwrap_or(&session.target);
        let ack = build_ack(
            &media.config.local_sip_uri,
            session,
            remote_tag,
            target,
            &format!("{}-ack", session.branch),
        )
        .map_err(|e| MediaError::MalformedSip(e.to_string()))?;
        return Ok(vec![MediaOutput::SendMessage(ack)]);
    }

    // If we already asked to stop/cancel the pending INVITE, acknowledge the 200 OK
    // (RFC 3261 requires this even if the response is malformed) and tear down the
    // accidental dialog before reporting failure.
    if state != SessionState::Inviting {
        let mut outputs = Vec::new();

        let session_ref = media
            .sessions
            .get(&sid)
            .ok_or(MediaError::SessionNotFound)?;
        let mut session = session_ref.clone();

        let remote_tag = remote_tag
            .or_else(|| session.remote_tag.clone())
            .unwrap_or_default();
        let contact = match contact {
            Ok(c) => c,
            Err(_) => session
                .remote_target
                .clone()
                .unwrap_or_else(|| session.target.clone()),
        };

        let ack_branch = format!("{}-ack-late", session.branch);
        let ack = build_ack(
            &media.config.local_sip_uri,
            &session,
            if remote_tag.is_empty() {
                None
            } else {
                Some(&remote_tag)
            },
            &contact,
            &ack_branch,
        )
        .map_err(|e| MediaError::MalformedSip(e.to_string()))?;
        outputs.push(MediaOutput::SendMessage(ack));

        if !remote_tag.is_empty() {
            session.remote_tag = Some(remote_tag);
            session.remote_target = Some(contact);
            session.cseq = session
                .cseq
                .checked_add(1)
                .ok_or_else(|| MediaError::InvalidState("CSeq overflow".to_string()))?;
            let bye_branch = format!("{}-bye-late", session.branch);
            let bye = if let Some(target) = session.remote_target.as_ref() {
                build_bye(
                    &media.config.local_sip_uri,
                    &session,
                    session.cseq,
                    &bye_branch,
                    target,
                )
                .map_err(|e| MediaError::MalformedSip(e.to_string()))
            } else {
                unreachable!("remote_target was just populated")
            }?;
            outputs.push(MediaOutput::SendMessage(bye));
        }

        let session = media
            .remove_session(sid)
            .ok_or(MediaError::SessionNotFound)?;
        outputs.push(MediaOutput::EmitEvent(failed_event(
            &session,
            &media.config.domain_id,
            "late 200 OK after stop",
        )));
        return Ok(outputs);
    }

    let remote_tag = remote_tag
        .ok_or_else(|| MediaError::MalformedSip("missing To tag in 200 OK".to_string()))?;
    let contact = contact?;

    let parsed_remote_sdp = cheetah_gb28181_core::parse_sdp(msg.body(), &REMOTE_SDP_CONFIG)
        .map_err(|e| MediaError::MalformedSdp(e.to_string()))?;

    // Enforce the compatibility profile's SDP payload/attribute allow-list before
    // committing to the dialog. Non-baseline payload types and vendor attributes
    // are rejected unless the matched profile enables the SDP override capability
    // and lists them. The 200 OK is still acknowledged and the dialog torn down.
    if let Some(reason) = validate_remote_sdp(&parsed_remote_sdp, &media.config.compatibility) {
        return reject_and_teardown(media, sid, &remote_tag, &contact, &reason);
    }

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

    // Validate the remote media address before mutating session state. A 200 OK
    // with an unparseable SDP connection address is treated as a failure. Per RFC
    // 3261 the response is still acknowledged and the accidental dialog is torn
    // down before the failure is reported.
    let source = match socket_addr(&remote_address, remote_port) {
        Ok(s) => s,
        Err(e) => {
            return reject_and_teardown(
                media,
                sid,
                &remote_tag,
                &contact,
                &format!("invalid SDP media address: {e}"),
            );
        }
    };

    let session = media
        .sessions
        .get(&sid)
        .ok_or(MediaError::SessionNotFound)?;
    let ack_branch = format!("{}-ack", session.branch);
    let ack = build_ack(
        &media.config.local_sip_uri,
        session,
        Some(&remote_tag),
        &contact,
        &ack_branch,
    )
    .map_err(|e| MediaError::MalformedSip(e.to_string()))?;

    let session = media
        .sessions
        .get_mut(&sid)
        .ok_or(MediaError::SessionNotFound)?;
    session.remote_tag = Some(remote_tag);
    session.remote_target = Some(contact);
    session.state = SessionState::Active;

    let event = Gb28181Event::MediaSessionStarted {
        domain_id: media.config.domain_id.clone(),
        media_session_id: session.media_session_id,
        channel_id: session.channel_id,
        device_id: session.device_id.clone(),
        source,
        remote_ssrc,
        remote_port,
        remote_proto,
    };

    Ok(vec![
        MediaOutput::SendMessage(ack),
        MediaOutput::EmitEvent(event),
    ])
}

/// Acknowledges the 200 OK, tears the accidental dialog down with a BYE and
/// reports the failure. Used when the device answer is unacceptable (bad media
/// address or a payload/attribute the compatibility profile does not allow).
fn reject_and_teardown(
    media: &mut Gb28181Media,
    sid: super::MediaSessionId,
    remote_tag: &str,
    contact: &SipUri,
    reason: &str,
) -> Result<Vec<MediaOutput>, MediaError> {
    let session = media
        .sessions
        .get(&sid)
        .ok_or(MediaError::SessionNotFound)?;
    let ack_branch = format!("{}-ack", session.branch);
    let ack = build_ack(
        &media.config.local_sip_uri,
        session,
        Some(remote_tag),
        contact,
        &ack_branch,
    )
    .map_err(|e| MediaError::MalformedSip(e.to_string()))?;

    let mut bye_session = session.clone();
    bye_session.remote_tag = Some(remote_tag.to_string());
    bye_session.cseq = bye_session
        .cseq
        .checked_add(1)
        .ok_or_else(|| MediaError::InvalidState("CSeq overflow".to_string()))?;
    let bye_branch = format!("{}-bye", session.branch);
    let bye = build_bye(
        &media.config.local_sip_uri,
        &bye_session,
        bye_session.cseq,
        &bye_branch,
        contact,
    )
    .map_err(|e| MediaError::MalformedSip(e.to_string()))?;

    let session = media
        .remove_session(sid)
        .ok_or(MediaError::SessionNotFound)?;
    Ok(vec![
        MediaOutput::SendMessage(ack),
        MediaOutput::SendMessage(bye),
        MediaOutput::EmitEvent(failed_event(&session, &media.config.domain_id, reason)),
    ])
}

/// Validates a parsed device SDP answer against the compatibility profile.
///
/// Returns `Some(reason)` for the first payload type or vendor attribute that is
/// not permitted, or `None` when every media description is acceptable. Only
/// vendor (`Unknown`) attributes are gated; typed attributes recognised by the
/// SDP parser are inherently standard and always accepted.
fn validate_remote_sdp(sdp: &SdpSession, profile: &CompatibilityProfile) -> Option<String> {
    for media in &sdp.media {
        for format in &media.formats {
            if !profile.sdp_payload_allowed(format) {
                return Some(format!(
                    "SDP payload type not permitted by profile: {format}"
                ));
            }
        }
        for attribute in &media.attributes {
            if let SdpAttribute::Unknown { name, .. } = attribute
                && !profile.sdp_attribute_allowed(name)
            {
                return Some(format!("SDP attribute not permitted by profile: {name}"));
            }
        }
    }
    None
}

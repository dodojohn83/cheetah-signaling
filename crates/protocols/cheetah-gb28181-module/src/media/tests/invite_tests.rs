use super::*;

#[test]
fn start_live_emits_invite_with_sdp() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    let outputs = media.process(MediaInput::Command(start_live(sid))).unwrap();
    assert_eq!(outputs.len(), 1);
    let MediaOutput::SendMessage(msg) = &outputs[0] else {
        panic!("expected SendMessage");
    };
    let SipMessage::Request {
        line,
        headers,
        body,
    } = msg
    else {
        panic!("expected request");
    };
    assert_eq!(line.method, Method::Invite);
    assert!(
        headers
            .get(&HeaderName::ContentType)
            .unwrap()
            .as_str()
            .contains("application/sdp")
    );
    assert!(!body.is_empty());
    let sdp = String::from_utf8_lossy(body);
    assert!(sdp.contains("m=video 5000 TCP/RTP/AVP 96"));
    assert!(sdp.contains("a=setup:passive"));
    assert!(sdp.contains("a=y:0200000000"));
}

#[test]
fn two_hundred_ok_triggers_ack_and_started_event() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();

    let ok = build_test_200_ok();
    let outputs = media.process(MediaInput::Message(ok)).unwrap();
    assert_eq!(outputs.len(), 2);
    let MediaOutput::SendMessage(ack) = &outputs[0] else {
        panic!("expected SendMessage ACK");
    };
    let SipMessage::Request { line, .. } = ack else {
        panic!("expected ACK request");
    };
    assert_eq!(line.method, Method::Ack);

    let MediaOutput::EmitEvent(Gb28181Event::MediaSessionStarted { remote_ssrc, .. }) = &outputs[1]
    else {
        panic!("expected MediaSessionStarted");
    };
    assert_eq!(remote_ssrc.as_deref(), Some("0200000001"));
}

#[test]
fn two_hundred_ok_with_invalid_media_address_removes_session_and_emits_failed() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();

    let ok = build_test_200_ok_with_connection("IN IP4 not-an-ip");
    let outputs = media.process(MediaInput::Message(ok)).unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(matches!(
        &outputs[0],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionFailed { .. })
    ));
    assert!(media.remove_session(sid).is_none());
}

#[test]
fn retransmitted_two_hundred_ok_reacknowledges_active_session() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();

    media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();

    let outputs = media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();
    assert_eq!(outputs.len(), 1);
    let MediaOutput::SendMessage(ack) = &outputs[0] else {
        panic!("expected SendMessage ACK");
    };
    let SipMessage::Request { line, headers, .. } = ack else {
        panic!("expected ACK request");
    };
    assert_eq!(line.method, Method::Ack);
    assert_eq!(
        line.uri.encode(),
        "sip:34020000001320000001@192.168.1.20:5061"
    );
    let cseq = headers.get(&HeaderName::CSeq).unwrap().as_str();
    assert!(cseq.starts_with("1 ACK"));
}

#[test]
fn retransmitted_invite_200_ok_matches_after_in_dialog_request() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();

    let outputs = media
        .process(MediaInput::Command(MediaCommand::ControlPlayback {
            media_session_id: sid,
            action: PlaybackAction::Play,
            scale: None,
            range: None,
        }))
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(matches!(&outputs[0], MediaOutput::SendMessage(_)));

    let outputs = media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();
    assert_eq!(outputs.len(), 1);
    let MediaOutput::SendMessage(ack) = &outputs[0] else {
        panic!("expected SendMessage ACK");
    };
    let SipMessage::Request { line, headers, .. } = ack else {
        panic!("expected ACK request");
    };
    assert_eq!(line.method, Method::Ack);
    let cseq = headers.get(&HeaderName::CSeq).unwrap().as_str();
    assert!(
        cseq.starts_with("1 ACK"),
        "re-ACK must use original INVITE CSeq"
    );
}

#[test]
fn stop_pending_invite_sends_cancel_and_does_not_corrupt_cseq() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();

    let outputs = media
        .process(MediaInput::Command(MediaCommand::StopMediaSession {
            media_session_id: sid,
        }))
        .unwrap();
    let MediaOutput::SendMessage(cancel) = &outputs[0] else {
        panic!("expected SendMessage");
    };
    let SipMessage::Request { line, headers, .. } = cancel else {
        panic!("expected request");
    };
    assert_eq!(line.method, Method::Cancel);
    let cseq = headers.get(&HeaderName::CSeq).unwrap().as_str();
    assert!(cseq.starts_with("1 CANCEL"));

    // Late 200 OK for the original INVITE must not establish a session.
    // It must still be ACKnowledged and BYE'd to tear down the accidental dialog.
    let outputs = media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();
    assert_eq!(outputs.len(), 3);

    let MediaOutput::SendMessage(ack) = &outputs[0] else {
        panic!("expected ACK SendMessage");
    };
    let SipMessage::Request {
        line: ack_line,
        headers: ack_headers,
        ..
    } = ack
    else {
        panic!("expected ACK request");
    };
    assert_eq!(ack_line.method, Method::Ack);
    let ack_cseq = ack_headers.get(&HeaderName::CSeq).unwrap().as_str();
    assert!(ack_cseq.starts_with("1 ACK"));

    let MediaOutput::SendMessage(bye) = &outputs[1] else {
        panic!("expected BYE SendMessage");
    };
    let SipMessage::Request {
        line: bye_line,
        headers: bye_headers,
        ..
    } = bye
    else {
        panic!("expected BYE request");
    };
    assert_eq!(bye_line.method, Method::Bye);
    let bye_cseq = bye_headers.get(&HeaderName::CSeq).unwrap().as_str();
    assert!(bye_cseq.starts_with("2 BYE"));

    assert!(matches!(
        &outputs[2],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionFailed { .. })
    ));
}

#[test]
fn cancel_response_after_invite_failure_is_ignored() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();

    // Stop while inviting sends CANCEL and leaves the session in Stopping.
    media
        .process(MediaInput::Command(MediaCommand::StopMediaSession {
            media_session_id: sid,
        }))
        .unwrap();

    // The INVITE is rejected first.
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bK1234"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 INVITE"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    let terminated = SipMessage::Response {
        line: StatusLine::new(487, "Request Terminated"),
        headers,
        body: Vec::new(),
    };
    let outputs = media.process(MediaInput::Message(terminated)).unwrap();
    assert!(matches!(
        &outputs[0],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionFailed { .. })
    ));

    // A late 200 OK for the CANCEL must be silently ignored, not an error.
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bK1234"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 CANCEL"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    let cancel_ok = SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    };
    let outputs = media.process(MediaInput::Message(cancel_ok)).unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn sip_message_builders_reject_crlf_in_branch() {
    use crate::media::invite::{build_ack, build_bye, build_cancel};
    use crate::media::session::{Session, SessionState};

    let local_uri = SipUri::parse("sip:server@192.168.1.10:5060").unwrap();
    let target = SipUri::parse("sip:device@192.168.1.20:5060").unwrap();
    let session = Session {
        media_session_id: MediaSessionId::generate(),
        channel_id: ChannelId::generate(),
        device_id: DeviceId::new("34020000001320000001").unwrap(),
        call_id: "call-1".to_string(),
        local_tag: "tag-local".to_string(),
        remote_tag: Some("tag-remote".to_string()),
        cseq: 2,
        invite_cseq: 1,
        branch: "z9hG4bKok".to_string(),
        target: target.clone(),
        remote_target: Some(target.clone()),
        state: SessionState::Active,
        media_address: "192.168.1.100".to_string(),
        media_port: 5000,
    };

    assert!(build_ack(&local_uri, &session, None, &target, "branch\r\n").is_err());
    assert!(build_bye(&local_uri, &session, 2, "branch\r\n", &target).is_err());
    assert!(build_cancel(&local_uri, &session, 1, "branch\r\n", &target).is_err());
}

#[test]
fn ack_uses_aor_for_to_and_contact_for_request_uri() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();

    let ok = build_test_200_ok();
    let outputs = media.process(MediaInput::Message(ok)).unwrap();

    let MediaOutput::SendMessage(ack) = &outputs[0] else {
        panic!("expected SendMessage ACK");
    };
    let SipMessage::Request { line, headers, .. } = ack else {
        panic!("expected request");
    };
    assert_eq!(
        line.uri.encode(),
        "sip:34020000001320000001@192.168.1.20:5061"
    );
    let to = headers.get(&HeaderName::To).expect("missing To").as_str();
    assert!(to.contains("sip:34020000001320000001@192.168.1.20:5060"));
    assert!(to.contains("tag=tag-remote"));
}

#[test]
fn malformed_contact_header_does_not_panic() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();

    let ok = build_test_200_ok_with(1, ">garbage<sip:x@y>");
    let result = media.process(MediaInput::Message(ok));
    assert!(matches!(result, Err(MediaError::MalformedSip(_))));
}

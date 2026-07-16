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

use super::*;

#[test]
fn start_playback_emits_invite_with_playback_sdp() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    let outputs = media
        .process(MediaInput::Command(start_playback(sid)))
        .unwrap();
    let MediaOutput::SendMessage(msg) = &outputs[0] else {
        panic!("expected SendMessage");
    };
    let SipMessage::Request { body, .. } = msg else {
        panic!("expected request");
    };
    let sdp = String::from_utf8_lossy(body);
    assert!(sdp.contains("s=Playback"));
    assert!(sdp.contains("t=1704067200 1704153600"));
    assert!(sdp.contains("a=y:0200000000"));
    assert!(sdp.contains("m=video 5000 RTP/AVP 96"));
}

#[test]
fn start_download_emits_invite_with_downloadspeed() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    let outputs = media
        .process(MediaInput::Command(start_download(sid)))
        .unwrap();
    let MediaOutput::SendMessage(msg) = &outputs[0] else {
        panic!("expected SendMessage");
    };
    let SipMessage::Request { body, .. } = msg else {
        panic!("expected request");
    };
    let sdp = String::from_utf8_lossy(body);
    assert!(sdp.contains("s=Download"));
    assert!(sdp.contains("a=downloadspeed:4"));
}

#[test]
fn start_talk_emits_audio_sendrecv_sdp() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    let outputs = media
        .process(MediaInput::Command(start_talk(sid, "G.711A")))
        .unwrap();
    let MediaOutput::SendMessage(msg) = &outputs[0] else {
        panic!("expected SendMessage");
    };
    let SipMessage::Request { body, .. } = msg else {
        panic!("expected request");
    };
    let sdp = String::from_utf8_lossy(body);
    assert!(sdp.contains("s=Talk"));
    assert!(sdp.contains("m=audio 5000 RTP/AVP 8"));
    assert!(sdp.contains("a=sendrecv"));
    assert!(sdp.contains("a=rtpmap:8 PCMA/8000"));
}

#[test]
fn start_talk_unsupported_codec_returns_unsupported() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    let result = media.process(MediaInput::Command(start_talk(sid, "AAC")));
    assert!(matches!(result, Err(MediaError::Unsupported(_))));
}

#[test]
fn start_broadcast_emits_audio_sendonly_sdp() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    let outputs = media
        .process(MediaInput::Command(start_broadcast(sid, "G.711A")))
        .unwrap();
    let MediaOutput::SendMessage(msg) = &outputs[0] else {
        panic!("expected SendMessage");
    };
    let SipMessage::Request { body, .. } = msg else {
        panic!("expected request");
    };
    let sdp = String::from_utf8_lossy(body);
    assert!(sdp.contains("s=Broadcast"));
    assert!(sdp.contains("m=audio 5000 RTP/AVP 8"));
    // Broadcast is one-way from the platform to the device.
    assert!(sdp.contains("a=sendonly"));
    assert!(sdp.contains("a=rtpmap:8 PCMA/8000"));
}

#[test]
fn start_broadcast_unsupported_codec_returns_unsupported_without_side_effects() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    let result = media.process(MediaInput::Command(start_broadcast(sid, "AAC")));
    assert!(matches!(result, Err(MediaError::Unsupported(_))));
    // The rejected broadcast must not create a session; a follow-up valid
    // broadcast for the same id must succeed.
    let ok = media.process(MediaInput::Command(start_broadcast(sid, "PCMU")));
    assert!(ok.is_ok());
}

#[test]
fn control_playback_emits_mansrtsp_info() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media
        .process(MediaInput::Command(start_playback(sid)))
        .unwrap();
    media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();

    let outputs = media
        .process(MediaInput::Command(MediaCommand::ControlPlayback {
            media_session_id: sid,
            action: PlaybackAction::Play,
            scale: Some(2.0),
            range: Some("clock=20240101T000000Z-20240101T010000Z".to_string()),
        }))
        .unwrap();
    assert_eq!(outputs.len(), 1);
    let MediaOutput::SendMessage(info) = &outputs[0] else {
        panic!("expected SendMessage");
    };
    let SipMessage::Request {
        line,
        headers,
        body,
        ..
    } = info
    else {
        panic!("expected request");
    };
    assert_eq!(line.method, Method::Info);
    let content_type = headers.get(&HeaderName::ContentType).unwrap().as_str();
    assert!(content_type.contains("application/MANSRTSP"));
    let body = String::from_utf8_lossy(body);
    assert!(body.contains("PLAY MANSRTSP/1.0"));
    assert!(body.contains("Scale: 2\r\n"));
    assert!(body.contains("Range: clock=20240101T000000Z-20240101T010000Z\r\n"));
    assert!(!body.contains("1.0\nScale")); // no bare LF
    assert!(body.ends_with("\r\n"));
}

#[test]
fn playback_control_branch_is_unique_per_request() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media
        .process(MediaInput::Command(start_playback(sid)))
        .unwrap();
    media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();

    let first = media
        .process(MediaInput::Command(MediaCommand::ControlPlayback {
            media_session_id: sid,
            action: PlaybackAction::Play,
            scale: Some(2.0),
            range: None,
        }))
        .unwrap();
    let second = media
        .process(MediaInput::Command(MediaCommand::ControlPlayback {
            media_session_id: sid,
            action: PlaybackAction::Play,
            scale: Some(4.0),
            range: None,
        }))
        .unwrap();

    let get_branch = |out: &[MediaOutput]| {
        let MediaOutput::SendMessage(SipMessage::Request { headers, .. }) = &out[0] else {
            panic!("expected request");
        };
        headers.get(&HeaderName::Via).unwrap().as_str().to_string()
    };
    let first_branch = get_branch(&first);
    let second_branch = get_branch(&second);
    assert_ne!(first_branch, second_branch);
}

#[test]
fn playback_control_rejects_range_injection() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media
        .process(MediaInput::Command(start_playback(sid)))
        .unwrap();
    media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();

    let result = media.process(MediaInput::Command(MediaCommand::ControlPlayback {
        media_session_id: sid,
        action: PlaybackAction::Play,
        scale: None,
        range: Some("clock=0-\r\nInjected: 1".to_string()),
    }));
    assert!(matches!(result, Err(MediaError::MalformedSip(_))));
}

#[test]
fn stop_active_session_rejects_cseq_overflow() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media
        .process(MediaInput::Command(start_live_with_cseq(sid, u32::MAX)))
        .unwrap();
    let ok = build_test_200_ok_with(u32::MAX, "<sip:34020000001320000001@192.168.1.20:5061>");
    media.process(MediaInput::Message(ok)).unwrap();

    let result = media.process(MediaInput::Command(MediaCommand::StopMediaSession {
        media_session_id: sid,
    }));
    assert!(matches!(result, Err(MediaError::InvalidState(_))));
}

#[test]
fn control_playback_rejects_cseq_overflow() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media
        .process(MediaInput::Command(start_live_with_cseq(sid, u32::MAX)))
        .unwrap();
    let ok = build_test_200_ok_with(u32::MAX, "<sip:34020000001320000001@192.168.1.20:5061>");
    media.process(MediaInput::Message(ok)).unwrap();

    let result = media.process(MediaInput::Command(MediaCommand::ControlPlayback {
        media_session_id: sid,
        action: PlaybackAction::Play,
        scale: None,
        range: None,
    }));
    assert!(matches!(result, Err(MediaError::InvalidState(_))));
}

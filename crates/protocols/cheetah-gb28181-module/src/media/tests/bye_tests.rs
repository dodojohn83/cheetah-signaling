use super::*;

#[test]
fn stop_live_sends_bye_and_removes_session_on_ok() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let ok = build_test_200_ok();
    media.process(MediaInput::Message(ok)).unwrap();

    let outputs = media
        .process(MediaInput::Command(MediaCommand::StopMediaSession {
            media_session_id: sid,
        }))
        .unwrap();
    assert_eq!(outputs.len(), 1);
    let MediaOutput::SendMessage(bye) = &outputs[0] else {
        panic!("expected BYE");
    };
    let SipMessage::Request { line, .. } = bye else {
        panic!("expected request");
    };
    assert_eq!(line.method, Method::Bye);

    let bye_ok = build_response_to_bye();
    let outputs = media.process(MediaInput::Message(bye_ok)).unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(matches!(
        &outputs[0],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionStopped { .. })
    ));
}

#[test]
fn device_bye_is_acknowledged_and_stops_session() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let ok = build_test_200_ok();
    media.process(MediaInput::Message(ok)).unwrap();

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.20:5061;branch=z9hG4bKbye"),
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
    headers.append(HeaderName::CSeq, HeaderValue::new("2 BYE"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    let bye = SipMessage::Request {
        line: RequestLine::new(
            Method::Bye,
            SipUri::parse("sip:server@192.168.1.10:5060").unwrap(),
        ),
        headers,
        body: Vec::new(),
    };

    let outputs = media.process(MediaInput::Message(bye)).unwrap();
    assert_eq!(outputs.len(), 2);
    let MediaOutput::SendMessage(ok) = &outputs[0] else {
        panic!("expected 200 OK response");
    };
    assert!(matches!(ok, SipMessage::Response { line, .. } if line.code == 200));
    assert!(matches!(
        &outputs[1],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionStopped { .. })
    ));
}

#[test]
fn bye_uses_contact_request_uri_and_aor_to() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let ok = build_test_200_ok();
    media.process(MediaInput::Message(ok)).unwrap();

    let outputs = media
        .process(MediaInput::Command(MediaCommand::StopMediaSession {
            media_session_id: sid,
        }))
        .unwrap();

    let MediaOutput::SendMessage(bye) = &outputs[0] else {
        panic!("expected SendMessage BYE");
    };
    let SipMessage::Request { line, headers, .. } = bye else {
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
fn call_index_cleaned_after_bye_response() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let ok = build_test_200_ok();
    media.process(MediaInput::Message(ok)).unwrap();
    media
        .process(MediaInput::Command(MediaCommand::StopMediaSession {
            media_session_id: sid,
        }))
        .unwrap();

    let bye_ok = build_response_to_bye();
    media.process(MediaInput::Message(bye_ok)).unwrap();

    // A second BYE response for the same Call-ID must no longer route to a session.
    let duplicate = build_response_to_bye();
    let outputs = media.process(MediaInput::Message(duplicate)).unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn bye_provisional_response_is_ignored() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();
    media
        .process(MediaInput::Command(MediaCommand::StopMediaSession {
            media_session_id: sid,
        }))
        .unwrap();

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bK1234-bye"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("2 BYE"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    let trying = SipMessage::Response {
        line: StatusLine::new(100, "Trying"),
        headers,
        body: Vec::new(),
    };

    let outputs = media.process(MediaInput::Message(trying)).unwrap();
    assert!(outputs.is_empty());

    let final_ok = build_response_to_bye();
    let outputs = media.process(MediaInput::Message(final_ok)).unwrap();
    assert!(matches!(
        &outputs[0],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionStopped { .. })
    ));
}

#[test]
fn invite_failure_cleans_call_index() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bK1234"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 INVITE"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    let busy = SipMessage::Response {
        line: StatusLine::new(486, "Busy Here"),
        headers,
        body: Vec::new(),
    };

    let outputs = media.process(MediaInput::Message(busy)).unwrap();
    assert!(matches!(
        &outputs[0],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionFailed { .. })
    ));

    let duplicate = build_response_to_bye();
    let outputs = media.process(MediaInput::Message(duplicate)).unwrap();
    assert!(outputs.is_empty());
}

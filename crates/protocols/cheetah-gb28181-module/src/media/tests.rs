//! Unit tests for the GB28181 media session state machine.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use super::*;
use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipUri, StatusLine,
};
use cheetah_signal_types::{ChannelId, MediaSessionId};

fn config() -> MediaConfig {
    MediaConfig {
        local_sip_uri: SipUri::parse("sip:server@192.168.1.10:5060").unwrap(),
        max_sessions: 8,
        domain_id: DomainId::new("3402000000").unwrap(),
    }
}

fn start_live_with_cseq(media_session_id: MediaSessionId, cseq: u32) -> MediaCommand {
    MediaCommand::StartLive {
        media_session_id,
        channel_id: ChannelId::generate(),
        device_id: DeviceId::new("34020000001320000001").unwrap(),
        target: SipUri::parse("sip:34020000001320000001@192.168.1.20:5060").unwrap(),
        call_id: "call-1".to_string(),
        local_tag: "tag-local".to_string(),
        cseq,
        branch: "z9hG4bK1234".to_string(),
        subject_session: "0200000000".to_string(),
        media_address: "192.168.1.100".to_string(),
        media_port: 5000,
        ssrc: "0200000000".to_string(),
        transport: MediaTransport::TcpPassive,
    }
}

fn start_live(media_session_id: MediaSessionId) -> MediaCommand {
    start_live_with_cseq(media_session_id, 1)
}

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

fn build_test_200_ok_with(cseq: u32, contact: &str) -> SipMessage {
    let sdp = "v=0\r\n\
              o=- 0 0 IN IP4 0.0.0.0\r\n\
              s=Play\r\n\
              c=IN IP4 192.168.1.200\r\n\
              t=0 0\r\n\
              m=video 6000 TCP/RTP/AVP 96\r\n\
              a=setup:active\r\n\
              a=connection:new\r\n\
              a=rtpmap:96 PS/90000\r\n\
              a=y:0200000001";
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
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{cseq} INVITE")));
    headers.append(HeaderName::Contact, HeaderValue::new(contact));
    headers.append(HeaderName::ContentType, HeaderValue::new("application/sdp"));
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(sdp.len().to_string()),
    );
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: sdp.as_bytes().to_vec(),
    }
}

fn build_test_200_ok() -> SipMessage {
    build_test_200_ok_with(1, "<sip:34020000001320000001@192.168.1.20:5061>")
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
    let result = media.process(MediaInput::Message(duplicate));
    assert!(matches!(result, Err(MediaError::SessionNotFound)));
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
    let result = media.process(MediaInput::Message(duplicate));
    assert!(matches!(result, Err(MediaError::SessionNotFound)));
}

fn build_response_to_bye() -> SipMessage {
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
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    }
}

fn start_playback(media_session_id: MediaSessionId) -> MediaCommand {
    MediaCommand::StartPlayback {
        media_session_id,
        channel_id: ChannelId::generate(),
        device_id: DeviceId::new("34020000001320000001").unwrap(),
        target: SipUri::parse("sip:34020000001320000001@192.168.1.20:5060").unwrap(),
        call_id: "call-1".to_string(),
        local_tag: "tag-local".to_string(),
        cseq: 1,
        branch: "z9hG4bK1234".to_string(),
        subject_session: "0200000000".to_string(),
        media_address: "192.168.1.100".to_string(),
        media_port: 5000,
        ssrc: "0200000000".to_string(),
        transport: MediaTransport::Udp,
        start_time: "1704067200".to_string(),
        end_time: "1704153600".to_string(),
    }
}

fn start_download(media_session_id: MediaSessionId) -> MediaCommand {
    MediaCommand::StartDownload {
        media_session_id,
        channel_id: ChannelId::generate(),
        device_id: DeviceId::new("34020000001320000001").unwrap(),
        target: SipUri::parse("sip:34020000001320000001@192.168.1.20:5060").unwrap(),
        call_id: "call-1".to_string(),
        local_tag: "tag-local".to_string(),
        cseq: 1,
        branch: "z9hG4bK1234".to_string(),
        subject_session: "0200000000".to_string(),
        media_address: "192.168.1.100".to_string(),
        media_port: 5000,
        ssrc: "0200000000".to_string(),
        transport: MediaTransport::Udp,
        start_time: "1704067200".to_string(),
        end_time: "1704153600".to_string(),
        download_speed: 4,
    }
}

fn start_talk(media_session_id: MediaSessionId, codec: &str) -> MediaCommand {
    MediaCommand::StartTalk {
        media_session_id,
        channel_id: ChannelId::generate(),
        device_id: DeviceId::new("34020000001320000001").unwrap(),
        target: SipUri::parse("sip:34020000001320000001@192.168.1.20:5060").unwrap(),
        call_id: "call-1".to_string(),
        local_tag: "tag-local".to_string(),
        cseq: 1,
        branch: "z9hG4bK1234".to_string(),
        subject_session: "0200000000".to_string(),
        media_address: "192.168.1.100".to_string(),
        media_port: 5000,
        codec: codec.to_string(),
        transport: MediaTransport::Udp,
    }
}

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
fn sdp_encoder_rejects_crlf_injection() {
    use cheetah_gb28181_core::sdp::{SdpOrigin, SdpSession, SdpTime};

    let session = SdpSession {
        version: "0".to_string(),
        origin: SdpOrigin {
            username: "-\r\ninject".to_string(),
            sess_id: "0".to_string(),
            sess_version: "0".to_string(),
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: "0.0.0.0".to_string(),
        },
        name: "Play".to_string(),
        times: vec![SdpTime {
            start: "0".to_string(),
            stop: "0".to_string(),
        }],
        ..Default::default()
    };
    let result = cheetah_gb28181_core::encode_sdp(&session);
    assert!(result.is_err());
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

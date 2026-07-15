//! Unit tests for the GB28181 media session state machine.

#![allow(clippy::unwrap_used)]

use super::*;
use cheetah_gb28181_core::{HeaderName, HeaderValue, RequestLine, SipHeaders, SipUri, StatusLine};
use cheetah_signal_types::{ChannelId, MediaSessionId};

fn config() -> MediaConfig {
    MediaConfig {
        local_sip_uri: SipUri::parse("sip:server@192.168.1.10:5060").unwrap(),
        max_sessions: 8,
        domain_id: DomainId::new("3402000000").unwrap(),
    }
}

fn start_live(media_session_id: MediaSessionId) -> MediaCommand {
    MediaCommand::StartLive {
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
        transport: MediaTransport::TcpPassive,
    }
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
fn stop_live_sends_bye_and_removes_session_on_ok() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let ok = build_test_200_ok();
    media.process(MediaInput::Message(ok)).unwrap();

    let outputs = media
        .process(MediaInput::Command(MediaCommand::StopLive {
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

fn build_test_200_ok() -> SipMessage {
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
    headers.append(HeaderName::CSeq, HeaderValue::new("1 INVITE"));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5061>"),
    );
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

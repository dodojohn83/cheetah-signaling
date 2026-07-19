//! GB28181 media state machine tests.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use super::*;
use cheetah_gb28181_core::{HeaderName, HeaderValue, Method, RequestLine, SipHeaders, StatusLine};

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

fn build_test_200_ok_with_connection(connection: &str) -> SipMessage {
    let sdp = format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 0.0.0.0\r\n\
         s=Play\r\n\
         c={connection}\r\n\
         t=0 0\r\n\
         m=video 6000 TCP/RTP/AVP 96\r\n\
         a=setup:active\r\n\
         a=connection:new\r\n\
         a=rtpmap:96 PS/90000\r\n\
         a=y:0200000001"
    );
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
        body: sdp.into_bytes(),
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

mod bye_tests;
mod invite_tests;
mod playback_tests;
mod sdp_tests;

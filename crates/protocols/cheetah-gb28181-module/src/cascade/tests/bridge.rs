//! Tests for the upstream play bridge handling in the GB28181 cascade.

use super::{
    config, local_uri, password_provider, register_to_connected, toggling_provider, upstream_uri,
};
use crate::cascade::{CascadeEvent, CascadeInput, CascadeOutput, Gb28181Cascade};
use crate::events::Gb28181Event;
use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn sample_sdp() -> &'static str {
    "v=0\r\n\
     o=- 0 0 IN IP4 10.0.0.1\r\n\
     s=Play\r\n\
     c=IN IP4 10.0.0.1\r\n\
     t=0 0\r\n\
     m=video 10000 RTP/AVP 96\r\n\
     a=rtpmap:96 PS/90000\r\n"
}

fn sample_answer_sdp() -> &'static str {
    "v=0\r\n\
     o=- 0 0 IN IP4 192.168.1.1\r\n\
     s=Play\r\n\
     c=IN IP4 192.168.1.1\r\n\
     t=0 0\r\n\
     m=video 20000 RTP/AVP 96\r\n\
     a=rtpmap:96 PS/90000\r\n"
}

fn build_invite(
    call_id: &str,
    target_user: &str,
    from_uri: &SipUri,
    from_tag: &str,
    body: &[u8],
) -> SipMessage {
    let target = SipUri::parse(format!("sip:{target_user}@example.com")).unwrap();
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-invite").unwrap(),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(from_uri, from_tag).unwrap(),
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&local_uri()));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(1, Method::Invite));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", from_uri.encode())),
    );
    headers.append(HeaderName::ContentType, HeaderValue::new("application/sdp"));
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));

    SipMessage::Request {
        line: RequestLine::new(Method::Invite, target),
        headers,
        body: body.to_vec(),
    }
}

fn build_ack(call_id: &str, from_tag: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-ack").unwrap(),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&upstream_uri(), from_tag).unwrap(),
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&local_uri()));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(2, Method::Ack));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));

    SipMessage::Request {
        line: RequestLine::new(Method::Ack, local_uri()),
        headers,
        body: Vec::new(),
    }
}

fn build_bye(call_id: &str, from_uri: &SipUri, from_tag: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-bye").unwrap(),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(from_uri, from_tag).unwrap(),
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&local_uri()));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(3, Method::Bye));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));

    SipMessage::Request {
        line: RequestLine::new(Method::Bye, local_uri()),
        headers,
        body: Vec::new(),
    }
}

fn build_cancel(call_id: &str, from_tag: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-invite").unwrap(),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&upstream_uri(), from_tag).unwrap(),
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&local_uri()));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(1, Method::Cancel));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));

    SipMessage::Request {
        line: RequestLine::new(Method::Cancel, local_uri()),
        headers,
        body: Vec::new(),
    }
}

#[test]
fn bridge_invite_while_unregistered_returns_403() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected SendResponse");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 403),
        _ => panic!("expected response"),
    }
}

#[test]
fn bridge_invite_emits_100_and_requested_event() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected 100 Trying response");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 100),
        _ => panic!("expected response"),
    };
    assert!(matches!(
        outputs[1],
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested { .. })
    ));
}

#[test]
fn bridge_media_ready_sends_200_ok_with_answer_sdp() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested { bridge_id, .. }) =
        &outputs[1]
    else {
        panic!("expected CascadePlayRequested");
    };

    let answer = sample_answer_sdp().to_string();
    let outputs = cascade
        .process(CascadeInput {
            now: 101,
            event: CascadeEvent::BridgeMediaReady {
                bridge_id: bridge_id.clone(),
                answer_sdp: answer.clone(),
            },
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected 200 OK response");
    };
    match resp {
        SipMessage::Response {
            line,
            headers,
            body,
        } => {
            assert_eq!(line.code, 200);
            assert!(headers.get(&HeaderName::ContentType).is_some());
            assert_eq!(body.as_slice(), answer.as_bytes());
        }
        _ => panic!("expected response"),
    };
}

#[test]
fn bridge_ack_then_media_stop_sends_bye() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested { bridge_id, .. }) =
        &outputs[1]
    else {
        panic!("expected CascadePlayRequested");
    };
    let bridge_id = bridge_id.clone();

    let answer = sample_answer_sdp().to_string();
    cascade
        .process(CascadeInput {
            now: 101,
            event: CascadeEvent::BridgeMediaReady {
                bridge_id: bridge_id.clone(),
                answer_sdp: answer,
            },
        })
        .unwrap();

    let ack = build_ack("call-1", "from-tag");
    cascade
        .process(CascadeInput {
            now: 102,
            event: CascadeEvent::Request(Box::new(ack)),
        })
        .unwrap();

    let outputs = cascade
        .process(CascadeInput {
            now: 103,
            event: CascadeEvent::BridgeMediaStop {
                bridge_id: bridge_id.clone(),
            },
        })
        .unwrap();

    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, CascadeOutput::SendRequest(_)))
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped { .. })
    )));

    // A second stop for the same bridge is idempotent.
    let outputs = cascade
        .process(CascadeInput {
            now: 104,
            event: CascadeEvent::BridgeMediaStop { bridge_id },
        })
        .unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn bridge_stop_before_ack_sends_bye_after_ack() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested { bridge_id, .. }) =
        &outputs[1]
    else {
        panic!("expected CascadePlayRequested");
    };
    let bridge_id = bridge_id.clone();

    let answer = sample_answer_sdp().to_string();
    cascade
        .process(CascadeInput {
            now: 101,
            event: CascadeEvent::BridgeMediaReady {
                bridge_id: bridge_id.clone(),
                answer_sdp: answer,
            },
        })
        .unwrap();

    // Application asks to stop before the upstream ACK arrives.
    let outputs = cascade
        .process(CascadeInput {
            now: 102,
            event: CascadeEvent::BridgeMediaStop {
                bridge_id: bridge_id.clone(),
            },
        })
        .unwrap();
    assert!(outputs.is_empty());

    // Once the upstream ACKs the 200 OK, the cascade immediately sends BYE.
    let ack = build_ack("call-1", "from-tag");
    let outputs = cascade
        .process(CascadeInput {
            now: 103,
            event: CascadeEvent::Request(Box::new(ack)),
        })
        .unwrap();

    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, CascadeOutput::SendRequest(_))),
        "expected BYE after ACK"
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped { .. })
    )));
}

#[test]
fn bridge_bye_from_upstream_tears_down() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested { bridge_id, .. }) =
        &outputs[1]
    else {
        panic!("expected CascadePlayRequested");
    };

    let answer = sample_answer_sdp().to_string();
    cascade
        .process(CascadeInput {
            now: 101,
            event: CascadeEvent::BridgeMediaReady {
                bridge_id: bridge_id.clone(),
                answer_sdp: answer,
            },
        })
        .unwrap();

    let ack = build_ack("call-1", "from-tag");
    cascade
        .process(CascadeInput {
            now: 102,
            event: CascadeEvent::Request(Box::new(ack)),
        })
        .unwrap();

    let bye = build_bye("call-1", &upstream_uri(), "from-tag");
    let outputs = cascade
        .process(CascadeInput {
            now: 103,
            event: CascadeEvent::Request(Box::new(bye)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected 200 OK response");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 200),
        _ => panic!("expected response"),
    };
    assert!(matches!(
        outputs[1],
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped { .. })
    ));
}

#[test]
fn bridge_cancel_while_invited_sends_487() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let cancel = build_cancel("call-1", "from-tag");
    let outputs = cascade
        .process(CascadeInput {
            now: 101,
            event: CascadeEvent::Request(Box::new(cancel)),
        })
        .unwrap();

    let mut saw_ok_without_empty_tag = false;
    for output in &outputs {
        if let CascadeOutput::SendResponse(SipMessage::Response { line, headers, .. }) = output
            && line.code == 200
        {
            let to = headers
                .get(&HeaderName::To)
                .map(|v| v.as_str())
                .unwrap_or("");
            assert!(!to.ends_with(";tag="), "To tag must not be empty");
            saw_ok_without_empty_tag = true;
        }
    }
    assert!(saw_ok_without_empty_tag);

    let codes: Vec<u16> = outputs
        .iter()
        .filter_map(|o| match o {
            CascadeOutput::SendResponse(SipMessage::Response { line, .. }) => Some(line.code),
            _ => None,
        })
        .collect();
    assert!(codes.contains(&200));
    assert!(codes.contains(&487));
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped { .. })
    )));
}

#[test]
fn bridge_invite_malformed_sdp_returns_400() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        b"not sdp",
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected SendResponse");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 400),
        _ => panic!("expected response"),
    };
}

#[test]
fn bridge_invite_from_unknown_returns_403() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let other = SipUri::parse("sip:other@other.example.com").unwrap();
    let msg = build_invite("call-1", "34020000001320000002", &other, "other-tag", body);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected SendResponse");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 403),
        _ => panic!("expected response"),
    };
}

#[test]
fn bridge_max_sessions_returns_486() {
    let mut cfg = config();
    cfg.media_bridge_max_sessions = 0;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).unwrap();
    register_to_connected(&mut cascade);

    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected SendResponse");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 486),
        _ => panic!("expected response"),
    };
}

#[test]
fn bridge_bye_from_unknown_upstream_is_ignored() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    // BYE from a different upstream must be ignored.
    let other = SipUri::parse("sip:other@other.example.com").unwrap();
    let bye = build_bye("call-1", &other, "other-tag");
    let outputs = cascade
        .process(CascadeInput {
            now: 101,
            event: CascadeEvent::Request(Box::new(bye)),
        })
        .unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn bridge_invited_timeout_sends_487_and_stops() {
    let mut cfg = config();
    cfg.media_bridge_transaction_timeout_seconds = 1;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let outputs = cascade
        .process(CascadeInput {
            now: 102,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected SendResponse");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 487),
        _ => panic!("expected response"),
    };
    assert!(matches!(
        outputs[1],
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped { .. })
    ));

    // The bridge is removed and a second tick is clean.
    let outputs = cascade
        .process(CascadeInput {
            now: 103,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn bridge_media_ready_after_cancel_is_noop() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested { bridge_id, .. }) =
        &outputs[1]
    else {
        panic!("expected CascadePlayRequested");
    };

    // Upstream cancels before the application finishes preparing media.
    let cancel = build_cancel("call-1", "from-tag");
    cascade
        .process(CascadeInput {
            now: 101,
            event: CascadeEvent::Request(Box::new(cancel)),
        })
        .unwrap();

    // The application's answer is now harmless.
    let answer = sample_answer_sdp().to_string();
    let outputs = cascade
        .process(CascadeInput {
            now: 102,
            event: CascadeEvent::BridgeMediaReady {
                bridge_id: bridge_id.clone(),
                answer_sdp: answer,
            },
        })
        .unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn bridge_invite_to_wrong_host_is_ignored() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let body = sample_sdp().as_bytes();
    let mut msg = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    // Point the request at a different host.
    if let SipMessage::Request { line, .. } = &mut msg {
        line.uri = SipUri::parse("sip:34020000001320000002@other.example.com").unwrap();
    }
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn refresh_preserves_bridge_cleanup_outputs_when_credentials_disappear() {
    let enabled = Arc::new(AtomicBool::new(true));
    let mut cfg = config();
    cfg.register_interval_seconds = 10;
    cfg.register_refresh_margin_seconds = 2;
    cfg.media_bridge_transaction_timeout_seconds = 3;
    let mut cascade = Gb28181Cascade::new(cfg, toggling_provider(enabled.clone())).unwrap();

    // Authenticated registration with a 10s expiry.
    let outputs = cascade
        .process(CascadeInput {
            now: 0,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = super::request_call_id_cseq(&outputs);

    let challenge = super::challenge_ctx().generate_challenge(0).unwrap();
    let response_401 = super::build_401(&challenge.to_header_value(), &call_id, &cseq);
    let outputs = cascade
        .process(CascadeInput {
            now: 1,
            event: CascadeEvent::Response(Box::new(response_401)),
        })
        .unwrap();
    let (call_id, cseq) = super::request_call_id_cseq(&outputs);

    cascade
        .process(CascadeInput {
            now: 2,
            event: CascadeEvent::Response(Box::new(super::build_200(10, &call_id, &cseq))),
        })
        .unwrap();

    // Incoming upstream INVITE.
    let body = sample_sdp().as_bytes();
    let invite = build_invite(
        "call-1",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        body,
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 4,
            event: CascadeEvent::Request(Box::new(invite)),
        })
        .unwrap();
    let CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested { bridge_id, .. }) =
        &outputs[1]
    else {
        panic!("expected CascadePlayRequested");
    };
    let bridge_id = bridge_id.clone();

    // Answer moves the bridge into Accepted; downstream stop before the
    // upstream ACK moves it into Closing.
    let answer = sample_answer_sdp().to_string();
    cascade
        .process(CascadeInput {
            now: 5,
            event: CascadeEvent::BridgeMediaReady {
                bridge_id: bridge_id.clone(),
                answer_sdp: answer,
            },
        })
        .unwrap();
    cascade
        .process(CascadeInput {
            now: 6,
            event: CascadeEvent::BridgeMediaStop { bridge_id },
        })
        .unwrap();

    // Credentials disappear before both bridge cleanup and refresh are due.
    enabled.store(false, Ordering::SeqCst);

    // At t=10 the closing bridge has expired and the registration refresh is
    // due.  The bridge cleanup event must be preserved even though the refresh
    // cannot be authenticated.
    let result = cascade.process(CascadeInput {
        now: 10,
        event: CascadeEvent::Tick,
    });
    assert!(
        result.is_ok(),
        "tick must not fail when refresh auth fails: {:?}",
        result
    );
    let outputs = result.unwrap();
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped { .. })
    )));
    assert!(
        outputs
            .iter()
            .all(|o| !matches!(o, CascadeOutput::SendRequest(_)))
    );
}

fn response_code(output: &CascadeOutput) -> Option<u16> {
    match output {
        CascadeOutput::SendResponse(SipMessage::Response { line, .. }) => Some(line.code),
        _ => None,
    }
}

#[test]
fn bridge_invite_rejected_when_sdp_address_violates_zone_policy() {
    use crate::endpoint_policy::EndpointPolicy;
    // `public_sip` rejects the private 10.0.0.1 media address in `sample_sdp`.
    let cfg = config().with_sdp_endpoint_policy(EndpointPolicy::public_sip());
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let msg = build_invite(
        "call-zone",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        sample_sdp().as_bytes(),
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(response_code(&outputs[0]), Some(400));
    // No bridge event is emitted for a rejected offer.
    assert!(
        outputs
            .iter()
            .all(|o| !matches!(o, CascadeOutput::EmitEvent(_)))
    );
}

#[test]
fn bridge_invite_accepted_when_sdp_address_matches_zone_policy() {
    use crate::endpoint_policy::EndpointPolicy;
    // `any_zone_sip` admits the private media address, so the bridge proceeds.
    let cfg = config().with_sdp_endpoint_policy(EndpointPolicy::any_zone_sip());
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let msg = build_invite(
        "call-zone-ok",
        "34020000001320000002",
        &upstream_uri(),
        "from-tag",
        sample_sdp().as_bytes(),
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();
    assert_eq!(response_code(&outputs[0]), Some(100));
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested { .. })
    )));
}

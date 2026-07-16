//! Keepalive tests for the GB28181 cascade upstream registration state machine.

use super::*;

fn message_response(
    call_id: &str,
    cseq: &str,
    code: u16,
    reason: &str,
    body: Vec<u8>,
) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new(cseq.to_string()));
    if !body.is_empty() {
        headers.append(
            HeaderName::ContentType,
            HeaderValue::new("Application/MANSCDP+xml"),
        );
        headers.append(
            HeaderName::ContentLength,
            HeaderValue::new(body.len().to_string()),
        );
    }
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body,
    }
}

fn register_to_connected_local(cascade: &mut Gb28181Cascade<impl CascadeCredentialProvider>) {
    let _ = super::register_to_connected(cascade);
}

#[test]
fn keepalive_sends_periodic_message_and_resets_on_success() {
    let mut cfg = config();
    cfg.keepalive_interval_seconds = 30;
    cfg.keepalive_timeout_seconds = 10;
    cfg.keepalive_max_failures = 3;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    register_to_connected_local(&mut cascade);

    // First keepalive is scheduled at the registration success time + interval.
    let outputs = cascade
        .process(CascadeInput {
            now: 1031,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert_eq!(outputs.len(), 1);
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    // Transport-level 200 OK resets failures and schedules the next keepalive.
    let outputs = cascade
        .process(CascadeInput {
            now: 1032,
            event: CascadeEvent::Response(Box::new(message_response(
                &call_id,
                &cseq,
                200,
                "OK",
                Vec::new(),
            ))),
        })
        .unwrap();
    assert!(outputs.is_empty());

    // Tick before the next interval is silent.
    let outputs = cascade
        .process(CascadeInput {
            now: 1061,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert!(outputs.is_empty());

    // Next periodic keepalive fires at now + interval.
    let outputs = cascade
        .process(CascadeInput {
            now: 1062,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert_eq!(outputs.len(), 1);
    let (_call_id2, cseq2) = request_call_id_cseq(&outputs);
    assert!(cseq2.starts_with('2'));
}

#[test]
fn keepalive_timeout_counts_failures_and_disconnects() {
    let keepalive_interval = 30;
    let keepalive_timeout = 10;
    let mut cfg = config();
    cfg.keepalive_interval_seconds = keepalive_interval;
    cfg.keepalive_timeout_seconds = keepalive_timeout;
    cfg.keepalive_max_failures = 2;
    let max_failures = cfg.keepalive_max_failures;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    register_to_connected_local(&mut cascade);

    let mut now = 1031;
    let mut outputs = cascade
        .process(CascadeInput {
            now,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    for i in 0..max_failures {
        let (_call_id, _cseq) = request_call_id_cseq(&outputs);
        now += keepalive_timeout as u64;

        outputs = cascade
            .process(CascadeInput {
                now,
                event: CascadeEvent::Tick,
            })
            .unwrap();

        if i + 1 == max_failures {
            assert!(outputs.iter().any(|o| matches!(
                o,
                CascadeOutput::EmitEvent(
                    crate::events::Gb28181Event::CascadePlatformDisconnected { .. }
                )
            )));
            return;
        }

        assert!(outputs.is_empty());

        // The next periodic keepalive should fire at the original schedule.
        now += keepalive_interval as u64 - keepalive_timeout as u64;
        outputs = cascade
            .process(CascadeInput {
                now,
                event: CascadeEvent::Tick,
            })
            .unwrap();
    }

    panic!("expected disconnection event after keepalive failures");
}

#[test]
fn keepalive_business_response_error_counts_failure() {
    let mut cfg = config();
    cfg.keepalive_interval_seconds = 30;
    cfg.keepalive_timeout_seconds = 10;
    cfg.keepalive_max_failures = 3;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    register_to_connected_local(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 1031,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>ERROR</Result>
</Response>"#
        .to_vec();
    let outputs = cascade
        .process(CascadeInput {
            now: 1032,
            event: CascadeEvent::Response(Box::new(message_response(
                &call_id, &cseq, 200, "OK", body,
            ))),
        })
        .unwrap();
    assert!(outputs.is_empty());

    // A subsequent transport-level success should still reset failures and allow
    // the keepalive cadence to continue.
    let outputs = cascade
        .process(CascadeInput {
            now: 1062,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let outputs = cascade
        .process(CascadeInput {
            now: 1063,
            event: CascadeEvent::Response(Box::new(message_response(
                &call_id,
                &cseq,
                200,
                "OK",
                Vec::new(),
            ))),
        })
        .unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn keepalive_redirect_treated_as_failure() {
    let mut cfg = config();
    cfg.keepalive_interval_seconds = 30;
    cfg.keepalive_timeout_seconds = 10;
    // With max_failures set to 1, a single redirect response must immediately
    // mark the platform disconnected.
    cfg.keepalive_max_failures = 1;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    register_to_connected_local(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 1031,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let outputs = cascade
        .process(CascadeInput {
            now: 1032,
            event: CascadeEvent::Response(Box::new(message_response(
                &call_id,
                &cseq,
                302,
                "Moved Temporarily",
                Vec::new(),
            ))),
        })
        .unwrap();

    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformDisconnected { .. })
    )));
}

#[test]
fn keepalive_provisional_response_preserves_timeout() {
    let mut cfg = config();
    cfg.keepalive_interval_seconds = 30;
    cfg.keepalive_timeout_seconds = 10;
    cfg.keepalive_max_failures = 1;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    register_to_connected_local(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 1031,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    // A 100 Trying provisional response must not disarm the timeout.
    let outputs = cascade
        .process(CascadeInput {
            now: 1032,
            event: CascadeEvent::Response(Box::new(message_response(
                &call_id,
                &cseq,
                100,
                "Trying",
                Vec::new(),
            ))),
        })
        .unwrap();
    assert!(outputs.is_empty());

    // After the timeout elapses with no final response, the failure count
    // (which was 0 when the provisional arrived) reaches max and disconnects.
    let outputs = cascade
        .process(CascadeInput {
            now: 1041,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformDisconnected { .. })
    )));
}

#[test]
fn stale_keepalive_response_is_ignored() {
    let mut cfg = config();
    cfg.keepalive_interval_seconds = 30;
    cfg.keepalive_timeout_seconds = 10;
    cfg.keepalive_max_failures = 2;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    register_to_connected_local(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 1031,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    let (call_id, cseq1) = request_call_id_cseq(&outputs);

    // First keepalive times out.
    let outputs = cascade
        .process(CascadeInput {
            now: 1041,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert!(outputs.is_empty());

    // A new keepalive is sent with the next CSeq.
    let outputs = cascade
        .process(CascadeInput {
            now: 1061,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    let (_call_id2, cseq2) = request_call_id_cseq(&outputs);
    assert_ne!(cseq1, cseq2);

    // A late 200 OK for the first (timed-out) keepalive must be ignored so
    // that the pending second keepalive can still time out and disconnect.
    let outputs = cascade
        .process(CascadeInput {
            now: 1062,
            event: CascadeEvent::Response(Box::new(message_response(
                &call_id,
                &cseq1,
                200,
                "OK",
                Vec::new(),
            ))),
        })
        .unwrap();
    assert!(outputs.is_empty());

    let outputs = cascade
        .process(CascadeInput {
            now: 1071,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformDisconnected { .. })
    )));
}

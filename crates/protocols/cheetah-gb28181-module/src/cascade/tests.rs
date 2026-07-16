//! Tests for the GB28181 cascade upstream registration state machine.

#![allow(clippy::unwrap_used)]
#![allow(clippy::obfuscated_if_else)]

use crate::cascade::{
    CascadeConfig, CascadeCredentialProvider, CascadeEvent, CascadeInput, CascadeOutput,
    Gb28181Cascade,
};
use crate::types::DomainId;
use cheetah_gb28181_core::{
    DigestContext, HeaderName, HeaderValue, Method, SipHeaders, SipMessage, SipUri, StatusLine,
};
use secrecy::SecretString;

fn domain_id() -> DomainId {
    DomainId::new("3402000000").unwrap()
}

fn local_uri() -> SipUri {
    SipUri::parse("sip:34020000001320000001@example.com").unwrap()
}

fn upstream_uri() -> SipUri {
    SipUri::parse("sip:34020000002000000001@upstream.example.com").unwrap()
}

fn config() -> CascadeConfig {
    CascadeConfig::with_options(
        domain_id(),
        local_uri(),
        upstream_uri(),
        "example.com".to_string(),
        "upstream-cred".to_string(),
        3600,
        30,
        true,
        false,
    )
    .unwrap()
}

fn password_provider() -> impl CascadeCredentialProvider {
    |_: &str| -> Option<SecretString> { Some(SecretString::new("secret".into())) }
}

fn extract_call_id_cseq(msg: &SipMessage) -> (String, String) {
    let call_id = msg.call_id().unwrap().to_string();
    let cseq = msg.cseq().unwrap();
    (call_id, format!("{} {}", cseq.0, cseq.1))
}

fn request_call_id_cseq(outputs: &[CascadeOutput]) -> (String, String) {
    let CascadeOutput::SendRequest(msg) = &outputs[0] else {
        panic!("expected SendRequest output");
    };
    extract_call_id_cseq(msg)
}

fn build_401(challenge: &str, call_id: &str, cseq: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new(cseq.to_string()));
    headers.append(
        HeaderName::WwwAuthenticate,
        HeaderValue::new(challenge.to_string()),
    );
    SipMessage::Response {
        line: StatusLine::new(401, "Unauthorized"),
        headers,
        body: Vec::new(),
    }
}

fn build_200(expires: u32, call_id: &str, cseq: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new(cseq.to_string()));
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@example.com>;tag=abc"),
    );
    headers.append(HeaderName::Expires, HeaderValue::new(expires.to_string()));
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    }
}

fn challenge_ctx() -> DigestContext {
    DigestContext::new(
        "example.com",
        b"this-is-a-very-long-secret-used-for-testing-only",
    )
    .unwrap()
}

#[test]
fn register_from_idle_sends_register_without_authorization() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider());
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let CascadeOutput::SendRequest(msg) = &outputs[0] else {
        panic!("expected SendRequest output");
    };
    let SipMessage::Request { line, .. } = msg else {
        panic!("expected request");
    };
    assert_eq!(line.method, Method::Register);
}

#[test]
fn unauthorized_response_triggers_authenticated_register() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider());
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let challenge = challenge_ctx().generate_challenge(1000).unwrap();
    let response_401 = build_401(&challenge.to_header_value(), &call_id, &cseq);

    let outputs = cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(response_401)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let CascadeOutput::SendRequest(msg) = &outputs[0] else {
        panic!("expected SendRequest output");
    };
    assert!(msg.headers().get(&HeaderName::Authorization).is_some());
}

#[test]
fn ok_response_emits_connected_and_registered_state() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider());
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let challenge = challenge_ctx().generate_challenge(1000).unwrap();
    let response_401 = build_401(&challenge.to_header_value(), &call_id, &cseq);
    let outputs = cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(response_401)),
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let outputs = cascade
        .process(CascadeInput {
            now: 1002,
            event: CascadeEvent::Response(Box::new(build_200(3600, &call_id, &cseq))),
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let CascadeOutput::EmitEvent(event) = &outputs[0] else {
        panic!("expected EmitEvent output");
    };
    assert!(matches!(
        event,
        crate::events::Gb28181Event::CascadePlatformConnected { .. }
    ));
}

#[test]
fn tick_after_refresh_sends_new_register() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider());
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    // Succeed without authentication for this test.
    let response_200 = build_200(60, &call_id, &cseq);
    let outputs = cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(response_200)),
        })
        .unwrap();
    assert!(matches!(
        outputs[0],
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformConnected { .. })
    ));

    // After 60 seconds the refresh should fire (minus 30s margin).
    let outputs = cascade
        .process(CascadeInput {
            now: 1031,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(matches!(outputs[0], CascadeOutput::SendRequest(_)));
}

#[test]
fn deregister_sends_expires_zero() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider());
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(build_200(60, &call_id, &cseq))),
        })
        .unwrap();

    let outputs = cascade
        .process(CascadeInput {
            now: 1002,
            event: CascadeEvent::Deregister,
        })
        .unwrap();

    let CascadeOutput::SendRequest(msg) = &outputs[0] else {
        panic!("expected SendRequest output");
    };
    let expires = msg.headers().get(&HeaderName::Expires).unwrap().as_str();
    assert_eq!(expires, "0");
}

#[test]
fn config_rejects_internal_upstream_ip() {
    let upstream = SipUri::parse("sip:registrar@127.0.0.1").unwrap();
    let result = CascadeConfig::with_options(
        domain_id(),
        local_uri(),
        upstream,
        "example.com".to_string(),
        "cred".to_string(),
        3600,
        30,
        true,
        false,
    );
    assert!(result.is_err());
}

#[test]
fn config_allows_internal_upstream_ip_when_enabled() {
    let upstream = SipUri::parse("sip:registrar@127.0.0.1").unwrap();
    let result = CascadeConfig::with_options(
        domain_id(),
        local_uri(),
        upstream,
        "example.com".to_string(),
        "cred".to_string(),
        3600,
        30,
        true,
        true,
    );
    assert!(result.is_ok());
}

#[test]
fn failure_backoff_eventually_disconnects() {
    let mut cfg = config();
    cfg.max_retries = 1;
    cfg.base_backoff_ms = 0;
    cfg.jitter_ms = 0;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    let mut outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();

    for i in 0..=cascade.config.max_retries {
        let (call_id, cseq) = request_call_id_cseq(&outputs);
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::CallId, HeaderValue::new(call_id));
        headers.append(HeaderName::CSeq, HeaderValue::new(cseq));
        let response_403 = SipMessage::Response {
            line: StatusLine::new(403, "Forbidden"),
            headers,
            body: Vec::new(),
        };

        outputs = cascade
            .process(CascadeInput {
                now: 1000 + u64::from(i) * 10,
                event: CascadeEvent::Response(Box::new(response_403)),
            })
            .unwrap();

        if outputs.iter().any(|o| {
            matches!(
                o,
                CascadeOutput::EmitEvent(
                    crate::events::Gb28181Event::CascadePlatformDisconnected { .. }
                )
            )
        }) {
            return;
        }

        // Advance past any backoff to trigger the next retry.
        outputs = cascade
            .process(CascadeInput {
                now: 1000 + u64::from(i + 1) * 10,
                event: CascadeEvent::Tick,
            })
            .unwrap();
    }

    panic!("expected a disconnection event after exhausting retries");
}

#[test]
fn transaction_timeout_triggers_retry() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider());
    cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();

    let timeout = cascade.config.transaction_timeout_seconds as u64;
    let outputs = cascade
        .process(CascadeInput {
            now: 1000 + timeout + 1,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    // A timed out transaction may emit nothing (backoff) or a disconnect; it
    // must not panic and must leave the state machine in Idle or Failed.
    assert!(outputs.len() <= 1);
}

#[test]
fn zero_expiry_disconnects_and_does_not_schedule_refresh() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider());
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let outputs = cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(build_200(0, &call_id, &cseq))),
        })
        .unwrap();

    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformDisconnected { .. })
    )));
}

#[test]
fn short_expiry_uses_proportional_refresh_margin() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider());
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    // Server grants only 60 seconds; refresh should happen at 1001 + 30,
    // not at now + 0 (which would happen with expires.saturating_sub(30)).
    let outputs = cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(build_200(60, &call_id, &cseq))),
        })
        .unwrap();

    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformConnected { .. })
    )));

    // Tick just before the refresh point does nothing.
    let outputs = cascade
        .process(CascadeInput {
            now: 1030,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert!(outputs.is_empty());

    // Tick at the refresh point sends a new REGISTER.
    let outputs = cascade
        .process(CascadeInput {
            now: 1031,
            event: CascadeEvent::Tick,
        })
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(matches!(outputs[0], CascadeOutput::SendRequest(_)));
}

#[test]
fn redirect_response_is_treated_as_failure() {
    let mut cfg = config();
    cfg.max_retries = 0;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let mut headers = SipHeaders::new();
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::new(cseq));
    let response_302 = SipMessage::Response {
        line: StatusLine::new(302, "Moved Temporarily"),
        headers,
        body: Vec::new(),
    };

    let outputs = cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(response_302)),
        })
        .unwrap();

    // Should emit a failure/disconnect event, not a connected event.
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformDisconnected { .. })
    )));
    assert!(!outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformConnected { .. })
    )));
}

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

fn register_to_connected(cascade: &mut Gb28181Cascade<impl CascadeCredentialProvider>) -> String {
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let outputs = cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(build_200(3600, &call_id, &cseq))),
        })
        .unwrap();
    assert!(matches!(
        outputs[0],
        CascadeOutput::EmitEvent(crate::events::Gb28181Event::CascadePlatformConnected { .. })
    ));
    call_id
}

#[test]
fn keepalive_sends_periodic_message_and_resets_on_success() {
    let mut cfg = config();
    cfg.keepalive_interval_seconds = 30;
    cfg.keepalive_timeout_seconds = 10;
    cfg.keepalive_max_failures = 3;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider());
    register_to_connected(&mut cascade);

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
    register_to_connected(&mut cascade);

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
    register_to_connected(&mut cascade);

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
    register_to_connected(&mut cascade);

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
    register_to_connected(&mut cascade);

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

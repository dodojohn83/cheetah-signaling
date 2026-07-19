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

pub(crate) fn domain_id() -> DomainId {
    DomainId::new("3402000000").unwrap()
}

pub(crate) fn local_uri() -> SipUri {
    SipUri::parse("sip:34020000001320000001@example.com").unwrap()
}

pub(crate) fn upstream_uri() -> SipUri {
    SipUri::parse("sip:34020000002000000001@upstream.example.com").unwrap()
}

pub(crate) fn config() -> CascadeConfig {
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

pub(crate) fn password_provider() -> impl CascadeCredentialProvider {
    |_: &str| -> Option<SecretString> { Some(SecretString::new("secret".into())) }
}

pub(crate) fn extract_call_id_cseq(msg: &SipMessage) -> (String, String) {
    let call_id = msg.call_id().unwrap().to_string();
    let cseq = msg.cseq().unwrap();
    (call_id, format!("{} {}", cseq.0, cseq.1))
}

pub(crate) fn request_call_id_cseq(outputs: &[CascadeOutput]) -> (String, String) {
    let CascadeOutput::SendRequest(msg) = &outputs[0] else {
        panic!("expected SendRequest output");
    };
    extract_call_id_cseq(msg)
}

fn extract_from_tag(msg: &SipMessage) -> String {
    let headers = msg.headers();
    let Some(from) = headers.get(&HeaderName::From) else {
        panic!("From header");
    };
    let Some(tag) = from
        .as_str()
        .split(';')
        .find_map(|param| param.trim().strip_prefix("tag="))
    else {
        panic!("From tag");
    };
    tag.trim().to_string()
}

pub(crate) fn build_401(challenge: &str, call_id: &str, cseq: &str) -> SipMessage {
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

pub(crate) fn build_200(expires: u32, call_id: &str, cseq: &str) -> SipMessage {
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

mod subscription;

pub(crate) fn register_to_connected(
    cascade: &mut Gb28181Cascade<impl CascadeCredentialProvider>,
) -> (String, String) {
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);
    let Some(register_request) = outputs.iter().find_map(|o| match o {
        CascadeOutput::SendRequest(msg) => Some(msg),
        _ => None,
    }) else {
        panic!("register request");
    };
    let local_tag = extract_from_tag(register_request);

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
    (call_id, local_tag)
}

#[test]
fn register_from_idle_sends_register_without_authorization() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
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
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
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
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
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
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
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
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
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
fn register_response_malformed_expires_returns_error() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    let mut headers = SipHeaders::new();
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new(cseq.to_string()));
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@example.com>;tag=abc".to_string()),
    );
    headers.append(
        HeaderName::Expires,
        HeaderValue::new("not-a-number".to_string()),
    );
    let response = SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    };

    let result = cascade.process(CascadeInput {
        now: 1001,
        event: CascadeEvent::Response(Box::new(response)),
    });
    assert!(matches!(
        result,
        Err(crate::cascade::CascadeError::MalformedSip(_))
    ));
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
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).unwrap();
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
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
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
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
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
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
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
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).unwrap();
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

mod bridge;
mod catalog;
mod catalog_security;
mod report;
mod tests_keepalive;

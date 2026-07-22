//! GB4-TST-002 cascade transition coverage driven through the public
//! [`Gb28181Cascade::process`] API: register / backoff / deregister / duplicate
//! and the internal-upstream ACL guard.
//!
//! The cascade state (`Idle`/`Registering`/`Registered`/`Failed`/`Deregistering`)
//! is private, so these tests observe transitions only through the emitted
//! [`CascadeOutput`]s, exactly as a real driver would. Subscription, bridge and
//! loop transitions require internal fixtures and are covered by the in-crate
//! suites under `cheetah-gb28181-module/src/cascade/tests`; this crate pins the
//! public register/backoff/deregister/ACL contract.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_gb28181_core::{HeaderName, HeaderValue, SipHeaders, SipMessage, SipUri, StatusLine};
use cheetah_gb28181_module::cascade::{
    CascadeConfig, CascadeCredentialProvider, CascadeEvent, CascadeInput, CascadeOutput,
    Gb28181Cascade,
};
use cheetah_gb28181_module::events::Gb28181Event;
use cheetah_gb28181_module::types::DomainId;
use secrecy::SecretString;

fn domain_id() -> DomainId {
    DomainId::new("3402000000").unwrap()
}

fn config() -> CascadeConfig {
    CascadeConfig::with_options(
        domain_id(),
        SipUri::parse("sip:34020000001320000001@example.com").unwrap(),
        SipUri::parse("sip:34020000002000000001@upstream.example.com").unwrap(),
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

fn request_call_id_cseq(outputs: &[CascadeOutput]) -> (String, String) {
    let CascadeOutput::SendRequest(msg) = &outputs[0] else {
        panic!("expected SendRequest output");
    };
    let call_id = msg.call_id().unwrap().to_string();
    let cseq = msg.cseq().unwrap();
    (call_id, format!("{} {}", cseq.0, cseq.1))
}

fn first_request(outputs: &[CascadeOutput]) -> &SipMessage {
    outputs
        .iter()
        .find_map(|o| match o {
            CascadeOutput::SendRequest(msg) => Some(msg),
            _ => None,
        })
        .expect("a SendRequest output")
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

fn register_to_connected(
    cascade: &mut Gb28181Cascade<impl CascadeCredentialProvider>,
) -> (String, String) {
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
    assert!(outputs.iter().any(|o| matches!(
        o,
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlatformConnected { .. })
    )));
    (call_id, cseq)
}

#[test]
fn register_from_idle_emits_register_request() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    assert!(matches!(outputs[0], CascadeOutput::SendRequest(_)));
}

#[test]
fn duplicate_register_while_registering_is_ignored() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    let first = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Register,
        })
        .unwrap();
    assert!(matches!(first[0], CascadeOutput::SendRequest(_)));
    // A second Register while the first attempt is still in flight must not
    // start a competing transaction.
    let second = cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Register,
        })
        .unwrap();
    assert!(
        second.is_empty(),
        "duplicate Register should be ignored, got {second:?}"
    );
}

#[test]
fn successful_registration_emits_connected() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
}

#[test]
fn deregister_from_connected_sends_expiry_zero_register() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);
    let outputs = cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Deregister,
        })
        .unwrap();
    let request = first_request(&outputs);
    let expires = request
        .headers()
        .get(&HeaderName::Expires)
        .map(|v| v.as_str().to_string());
    assert_eq!(expires.as_deref(), Some("0"));
}

#[test]
fn repeated_failures_backoff_then_disconnect() {
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

    let mut disconnected = false;
    for i in 0..5u64 {
        let (call_id, cseq) = request_call_id_cseq(&outputs);
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::CallId, HeaderValue::new(call_id));
        headers.append(HeaderName::CSeq, HeaderValue::new(cseq));
        let response_403 = SipMessage::Response {
            line: StatusLine::new(403, "Forbidden"),
            headers,
            body: Vec::new(),
        };
        let step = cascade
            .process(CascadeInput {
                now: 1000 + i * 10,
                event: CascadeEvent::Response(Box::new(response_403)),
            })
            .unwrap();
        if step.iter().any(|o| {
            matches!(
                o,
                CascadeOutput::EmitEvent(Gb28181Event::CascadePlatformDisconnected { .. })
            )
        }) {
            disconnected = true;
            break;
        }
        // Advance past any backoff window to trigger the next retry.
        outputs = cascade
            .process(CascadeInput {
                now: 2000 + i * 10,
                event: CascadeEvent::Tick,
            })
            .unwrap();
        if outputs.is_empty() {
            break;
        }
    }
    assert!(disconnected, "exhausted retries must emit a disconnect");
}

/// ACL: an internal-IP upstream is rejected unless explicitly enabled.
#[test]
fn internal_upstream_is_rejected_unless_enabled() {
    let upstream = SipUri::parse("sip:registrar@127.0.0.1").unwrap();
    let rejected = CascadeConfig::with_options(
        domain_id(),
        SipUri::parse("sip:34020000001320000001@example.com").unwrap(),
        upstream.clone(),
        "example.com".to_string(),
        "upstream-cred".to_string(),
        3600,
        30,
        true,
        false,
    );
    assert!(rejected.is_err(), "internal upstream must be rejected");

    let allowed = CascadeConfig::with_options(
        domain_id(),
        SipUri::parse("sip:34020000001320000001@example.com").unwrap(),
        upstream,
        "example.com".to_string(),
        "upstream-cred".to_string(),
        3600,
        30,
        true,
        true,
    );
    assert!(allowed.is_ok(), "internal upstream allowed when opted in");
}

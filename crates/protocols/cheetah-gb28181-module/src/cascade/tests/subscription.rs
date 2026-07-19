//! Tests for the upstream event subscription manager.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use crate::cascade::tests::{
    build_200, config, password_provider, register_to_connected, request_call_id_cseq,
};
use crate::cascade::{CascadeConfig, CascadeEvent, CascadeInput, CascadeOutput, Gb28181Cascade};
use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri, StatusLine,
};

fn test_config() -> CascadeConfig {
    let mut cfg = config();
    // Push keepalive far into the future so subscription tick tests do not
    // produce unrelated keepalive MESSAGE requests.
    cfg.keepalive_interval_seconds = 1_000_000;
    cfg
}

fn subscribe_request(event: &str, expires: u64) -> SipMessage {
    subscribe_request_with_call_id(event, Some(expires), "sub-call-id-1")
}

fn subscribe_request_with_call_id(event: &str, expires: Option<u64>, call_id: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-initial").unwrap(),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:34020000002000000001@upstream.example.com>;tag=remote-tag"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@example.com>"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 SUBSCRIBE"));
    headers.append(
        HeaderName::Other("Event".to_string()),
        HeaderValue::new(event.to_string()),
    );
    if let Some(expires) = expires {
        headers.append(HeaderName::Expires, HeaderValue::new(expires.to_string()));
    }
    headers.append(
        HeaderName::Contact,
        HeaderValue::new("<sip:34020000002000000001@upstream.example.com>"),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));

    SipMessage::Request {
        line: RequestLine::new(
            Method::Subscribe,
            SipUri::parse("sip:34020000001320000001@example.com").unwrap(),
        ),
        headers,
        body: Vec::new(),
    }
}

fn subscribe_request_without_expires(event: &str) -> SipMessage {
    subscribe_request_with_call_id(event, None, "sub-call-id-1")
}

fn notify_response_ok(call_id: &str, cseq: u32) -> SipMessage {
    notify_response(200, "OK", call_id, cseq)
}

fn notify_response(code: u16, reason: &str, call_id: &str, cseq: u32) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{cseq} NOTIFY")));
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:34020000002000000001@upstream.example.com>;tag=remote-tag"),
    );
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: Vec::new(),
    }
}

fn first_request(outputs: &[CascadeOutput]) -> Option<&SipMessage> {
    outputs.iter().find_map(|o| match o {
        CascadeOutput::SendRequest(msg) => Some(msg),
        _ => None,
    })
}

fn first_response(outputs: &[CascadeOutput]) -> Option<&SipMessage> {
    outputs.iter().find_map(|o| match o {
        CascadeOutput::SendResponse(msg) => Some(msg),
        _ => None,
    })
}

#[test]
fn subscribe_creates_subscription_and_sends_initial_notify() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Catalog", 60))),
        })
        .unwrap();

    let response = first_response(&outputs).expect("200 OK response");
    assert!(matches!(response, SipMessage::Response { line, .. } if line.code == 200));
    assert_eq!(
        response
            .headers()
            .get(&HeaderName::Expires)
            .map(|v| v.as_str()),
        Some("60")
    );

    let notify = first_request(&outputs).expect("NOTIFY request");
    assert!(matches!(notify, SipMessage::Request { line, .. } if line.method == Method::Notify));
    assert_eq!(
        notify
            .headers()
            .get(&HeaderName::Other("Event".to_string()))
            .map(|v| v.as_str()),
        Some("Catalog")
    );
    assert!(
        notify
            .headers()
            .get(&HeaderName::Other("Subscription-State".to_string()))
            .is_some_and(|v| v.as_str().starts_with("active"))
    );
}

#[test]
fn subscribe_refresh_updates_expiry_and_does_not_duplicate() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Alarm", 60))),
        })
        .unwrap();

    let outputs = cascade
        .process(CascadeInput {
            now: 2005,
            event: CascadeEvent::Request(Box::new(subscribe_request("Alarm", 120))),
        })
        .unwrap();

    let response = first_response(&outputs).expect("200 OK response");
    assert!(matches!(response, SipMessage::Response { line, .. } if line.code == 200));
    assert_eq!(
        response
            .headers()
            .get(&HeaderName::Expires)
            .map(|v| v.as_str()),
        Some("120")
    );
    assert_eq!(cascade.subscriptions.len(), 1);
}

#[test]
fn subscribe_with_bad_event_returns_489() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Presence", 60))),
        })
        .unwrap();

    let response = first_response(&outputs).expect("response");
    assert!(matches!(response, SipMessage::Response { line, .. } if line.code == 489));
    assert!(cascade.subscriptions.is_empty());
}

#[test]
fn subscribe_with_expires_zero_terminates_subscription() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("MobilePosition", 60))),
        })
        .unwrap();

    assert_eq!(cascade.subscriptions.len(), 1);

    let outputs = cascade
        .process(CascadeInput {
            now: 2005,
            event: CascadeEvent::Request(Box::new(subscribe_request_with_call_id(
                "MobilePosition",
                Some(0),
                "sub-call-id-1",
            ))),
        })
        .unwrap();

    let response = first_response(&outputs).expect("200 OK response");
    assert!(matches!(response, SipMessage::Response { line, .. } if line.code == 200));
    assert_eq!(
        response
            .headers()
            .get(&HeaderName::Expires)
            .map(|v| v.as_str()),
        Some("0")
    );
    let notify = first_request(&outputs).expect("final NOTIFY");
    assert!(
        notify
            .headers()
            .get(&HeaderName::Other("Subscription-State".to_string()))
            .is_some_and(|v| v.as_str().starts_with("terminated"))
    );
    assert!(cascade.subscriptions.is_empty());
}

#[test]
fn notify_response_clears_pending_retransmission() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Catalog", 60))),
        })
        .unwrap();

    let notify = first_request(&outputs).expect("NOTIFY request");
    let (cseq, call_id) = (
        notify.cseq().unwrap().0,
        notify.call_id().unwrap().to_string(),
    );

    cascade
        .process(CascadeInput {
            now: 2001,
            event: CascadeEvent::Response(Box::new(notify_response_ok(&call_id, cseq))),
        })
        .unwrap();
    let outputs = cascade
        .process(CascadeInput {
            now: 2001 + 5,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    assert!(outputs.iter().find_map(|o| match o {
        CascadeOutput::SendRequest(msg)
            if matches!(msg, SipMessage::Request { line, .. } if line.method == Method::Notify) =>
        {
            Some(msg)
        }
        _ => None,
    }).is_none());
}

#[test]
fn subscription_expires_and_sends_final_notify_on_tick() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Catalog", 60))),
        })
        .unwrap();

    let outputs = cascade
        .process(CascadeInput {
            now: 2061,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    let notify = first_request(&outputs).expect("final NOTIFY");
    assert!(
        notify
            .headers()
            .get(&HeaderName::Other("Subscription-State".to_string()))
            .is_some_and(|v| v.as_str().starts_with("terminated"))
    );
    assert!(cascade.subscriptions.is_empty());
}

#[test]
fn subscribe_without_expires_uses_configured_default() {
    let mut cfg = test_config();
    cfg.subscription_default_expiry_seconds = 1234;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).unwrap();
    register_to_connected(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request_without_expires("Catalog"))),
        })
        .unwrap();

    let response = first_response(&outputs).expect("200 OK response");
    assert!(matches!(response, SipMessage::Response { line, .. } if line.code == 200));
    assert_eq!(
        response
            .headers()
            .get(&HeaderName::Expires)
            .map(|v| v.as_str()),
        Some("1234")
    );
}

#[test]
fn refresh_preserves_local_tag_when_upstream_omits_to_tag() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Catalog", 60))),
        })
        .unwrap();

    let first_local_tag = cascade
        .subscriptions
        .values()
        .next()
        .unwrap()
        .local_tag()
        .to_string();

    cascade
        .process(CascadeInput {
            now: 2005,
            event: CascadeEvent::Request(Box::new(subscribe_request("Catalog", 120))),
        })
        .unwrap();

    let second_local_tag = cascade
        .subscriptions
        .values()
        .next()
        .unwrap()
        .local_tag()
        .to_string();
    assert_eq!(first_local_tag, second_local_tag);
}

#[test]
fn subscriptions_cleared_on_deregister() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Catalog", 60))),
        })
        .unwrap();

    assert_eq!(cascade.subscriptions.len(), 1);

    let outputs = cascade
        .process(CascadeInput {
            now: 2010,
            event: CascadeEvent::Deregister,
        })
        .unwrap();
    let (call_id, cseq) = request_call_id_cseq(&outputs);

    cascade
        .process(CascadeInput {
            now: 2011,
            event: CascadeEvent::Response(Box::new(build_200(0, &call_id, &cseq))),
        })
        .unwrap();

    assert!(cascade.subscriptions.is_empty());
}

#[test]
fn refresh_with_different_event_package_returns_489() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Catalog", 60))),
        })
        .unwrap();
    assert_eq!(cascade.subscriptions.len(), 1);

    let outputs = cascade
        .process(CascadeInput {
            now: 2005,
            event: CascadeEvent::Request(Box::new(subscribe_request("Alarm", 60))),
        })
        .unwrap();

    let response = first_response(&outputs).expect("489 response");
    assert!(matches!(response, SipMessage::Response { line, .. } if line.code == 489));
    assert_eq!(cascade.subscriptions.len(), 1);
}

#[test]
fn error_response_to_notify_terminates_subscription() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request("Catalog", 60))),
        })
        .unwrap();
    let notify = first_request(&outputs).expect("initial NOTIFY");
    let call_id = notify.call_id().unwrap().to_string();
    let cseq = notify.cseq().unwrap().0;

    cascade
        .process(CascadeInput {
            now: 2001,
            event: CascadeEvent::Response(Box::new(notify_response(
                481,
                "Call/Transaction Does Not Exist",
                &call_id,
                cseq,
            ))),
        })
        .unwrap();

    assert!(cascade.subscriptions.is_empty());
}

#[test]
fn capacity_eviction_sends_terminated_notify() {
    let mut cfg = test_config();
    cfg.subscription_max_subscriptions = 1;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).unwrap();
    register_to_connected(&mut cascade);

    cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(subscribe_request_with_call_id(
                "Catalog",
                Some(60),
                "sub-call-id-1",
            ))),
        })
        .unwrap();
    assert_eq!(cascade.subscriptions.len(), 1);

    let outputs = cascade
        .process(CascadeInput {
            now: 2005,
            event: CascadeEvent::Request(Box::new(subscribe_request_with_call_id(
                "Alarm",
                Some(60),
                "sub-call-id-2",
            ))),
        })
        .unwrap();

    assert_eq!(cascade.subscriptions.len(), 1);
    let mut requests = outputs.iter().filter_map(|o| match o {
        CascadeOutput::SendRequest(msg) => Some(msg),
        _ => None,
    });
    let terminated = requests.next().expect("terminated NOTIFY for evicted sub");
    assert!(
        terminated
            .headers()
            .get(&HeaderName::Other("Subscription-State".to_string()))
            .is_some_and(|v| v.as_str().starts_with("terminated"))
    );
    assert!(
        terminated
            .headers()
            .get(&HeaderName::Other("Event".to_string()))
            .is_some_and(|v| v.as_str().eq_ignore_ascii_case("Catalog"))
    );

    let active = requests.next().expect("active NOTIFY for new sub");
    assert!(
        active
            .headers()
            .get(&HeaderName::Other("Subscription-State".to_string()))
            .is_some_and(|v| v.as_str().starts_with("active"))
    );
}

#[test]
fn subscribe_with_malformed_expires_returns_400() {
    let mut cascade = Gb28181Cascade::new(test_config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    let mut request = subscribe_request_with_call_id("Catalog", None, "sub-call-id-1");
    request
        .headers_mut()
        .append(HeaderName::Expires, HeaderValue::new("not-a-number"));

    let outputs = cascade
        .process(CascadeInput {
            now: 2000,
            event: CascadeEvent::Request(Box::new(request)),
        })
        .unwrap();

    let response = first_response(&outputs).expect("400 response");
    assert!(matches!(response, SipMessage::Response { line, .. } if line.code == 400));
    assert!(cascade.subscriptions.is_empty());
}

//! Tests for the upstream event subscription manager.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use crate::cascade::tests::{config, password_provider, register_to_connected};
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
    let mut headers = SipHeaders::new();
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{cseq} NOTIFY")));
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:34020000002000000001@upstream.example.com>;tag=remote-tag"),
    );
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
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

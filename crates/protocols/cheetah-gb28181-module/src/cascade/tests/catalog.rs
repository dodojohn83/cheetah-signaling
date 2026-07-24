//! Tests for GB28181 cascade catalog sharing.

use std::sync::Arc;

use crate::cascade::catalog::{
    CatalogError, CatalogFilter, CatalogPage, CatalogProvider, CatalogQuery,
};
use crate::cascade::{CascadeEvent, CascadeInput, CascadeOutput, Gb28181Cascade};
use crate::xml::catalog::{CatalogItem, parse_catalog};
use cheetah_gb28181_core::{
    Body, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
};

use super::{config, local_uri, password_provider, register_to_connected, upstream_uri};

pub(crate) fn catalog_query_message(sn: &str, device_id: &str, call_id: &str) -> SipMessage {
    let body = format!(
        r#"<?xml version="1.0"?>
<Query>
    <CmdType>Catalog</CmdType>
    <SN>{sn}</SN>
    <DeviceID>{device_id}</DeviceID>
</Query>"#
    );
    let body_bytes: Body = body.into_bytes();

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-abc").unwrap(),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&upstream_uri(), "upstream-tag").unwrap(),
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&local_uri()));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(1, Method::Message));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body_bytes.len().to_string()),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));

    SipMessage::Request {
        line: RequestLine::new(Method::Message, local_uri()),
        headers,
        body: body_bytes,
    }
}

struct TestItem {
    item: CatalogItem,
    tenant_id: String,
    tags: Vec<String>,
    org_path: String,
}

struct TestCatalogProvider {
    all: Vec<TestItem>,
}

impl CatalogProvider for TestCatalogProvider {
    fn query_page(
        &self,
        query: &CatalogQuery,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<CatalogPage, CatalogError> {
        let filtered: Vec<CatalogItem> = self
            .all
            .iter()
            .filter(|i| {
                if let Some(tenant) = &query.filter.tenant_id
                    && i.tenant_id != *tenant
                {
                    return false;
                }
                if !query.filter.whitelisted_device_ids.is_empty()
                    && !query
                        .filter
                        .whitelisted_device_ids
                        .contains(&i.item.device_id)
                {
                    return false;
                }
                if !query.filter.tags.is_empty()
                    && !query.filter.tags.iter().any(|t| i.tags.contains(t))
                {
                    return false;
                }
                if let Some(prefix) = &query.filter.org_path_prefix
                    && !i.org_path.starts_with(prefix)
                {
                    return false;
                }
                true
            })
            .map(|i| i.item.clone())
            .collect();
        let offset = cursor.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
        let total = filtered.len();
        let items: Vec<CatalogItem> = filtered.into_iter().skip(offset).take(limit).collect();
        let next_cursor = if offset.saturating_add(items.len()) < total {
            Some(offset.saturating_add(limit).to_string())
        } else {
            None
        };
        Ok(CatalogPage {
            items,
            total,
            next_cursor,
        })
    }
}

fn make_item(device_id: &str, tenant: &str, tag: &str, org: &str) -> TestItem {
    TestItem {
        item: CatalogItem {
            device_id: device_id.to_string(),
            name: Some(format!("Camera {device_id}")),
            status: Some("ON".to_string()),
            ..Default::default()
        },
        tenant_id: tenant.to_string(),
        tags: vec![tag.to_string()],
        org_path: org.to_string(),
    }
}

fn setup_provider() -> Arc<TestCatalogProvider> {
    Arc::new(TestCatalogProvider {
        all: vec![
            make_item("34020000001320000001", "t1", "outdoor", "/t1/floor1"),
            make_item("34020000001320000002", "t1", "indoor", "/t1/floor1"),
            make_item("34020000001320000003", "t2", "outdoor", "/t2/floor1"),
            make_item("34020000001320000004", "t1", "outdoor", "/t1/floor2"),
            make_item("34020000001320000005", "t1", "outdoor", "/t1/floor2"),
        ],
    })
}

fn catalog_response_bodies(outputs: &[CascadeOutput]) -> Vec<String> {
    outputs
        .iter()
        .filter_map(|o| match o {
            CascadeOutput::SendRequest(msg) => Some(msg),
            _ => None,
        })
        .map(|msg| String::from_utf8_lossy(msg.body()).to_string())
        .collect()
}

#[test]
fn catalog_query_without_provider_is_ignored() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    let (call_id, _) = register_to_connected(&mut cascade);
    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn non_message_request_is_ignored() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 2;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let _ = register_to_connected(&mut cascade);

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-abc").unwrap(),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    let msg = SipMessage::Request {
        line: RequestLine::new(Method::Register, local_uri()),
        headers,
        body: Vec::new(),
    };

    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();
    assert!(outputs.is_empty());
}

#[test]
fn catalog_query_returns_ok_and_paginated_messages() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 2;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("7", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert!(!outputs.is_empty());
    assert!(matches!(outputs[0], CascadeOutput::SendResponse(_)));
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!();
    };
    assert_eq!(resp.body().len(), 0);
    assert!(resp.call_id().is_some());

    let bodies = catalog_response_bodies(&outputs);
    assert_eq!(bodies.len(), 3);

    let first = parse_catalog(bodies[0].as_bytes()).unwrap();
    assert_eq!(first.sn, "7");
    assert_eq!(first.device_id, "34020000001320000001");
    assert_eq!(first.sum_num, 5);
    assert_eq!(first.num, 2);
    assert_eq!(first.items.len(), 2);

    let second = parse_catalog(bodies[1].as_bytes()).unwrap();
    assert_eq!(second.sum_num, 5);
    assert_eq!(second.num, 2);
    assert_eq!(second.items.len(), 2);

    let third = parse_catalog(bodies[2].as_bytes()).unwrap();
    assert_eq!(third.sum_num, 5);
    assert_eq!(third.num, 1);
    assert_eq!(third.items.len(), 1);
}

#[test]
fn catalog_filter_respects_tenant_and_whitelist() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    cfg.catalog_filter = CatalogFilter {
        tenant_id: Some("t1".to_string()),
        whitelisted_device_ids: vec![
            "34020000001320000001".to_string(),
            "34020000001320000002".to_string(),
            "34020000001320000005".to_string(),
        ],
        ..Default::default()
    };
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let bodies = catalog_response_bodies(&outputs);
    assert_eq!(bodies.len(), 1);
    let parsed = parse_catalog(bodies[0].as_bytes()).unwrap();
    assert_eq!(parsed.sum_num, 3);
    assert_eq!(parsed.items.len(), 3);
    assert!(
        parsed
            .items
            .iter()
            .all(|i| i.device_id.starts_with("3402000000132000000"))
    );
}

#[test]
fn catalog_filter_respects_tags_and_org_prefix() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    cfg.catalog_filter = CatalogFilter {
        tags: vec!["outdoor".to_string()],
        org_path_prefix: Some("/t1".to_string()),
        ..Default::default()
    };
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let bodies = catalog_response_bodies(&outputs);
    assert_eq!(bodies.len(), 1);
    let parsed = parse_catalog(bodies[0].as_bytes()).unwrap();
    assert_eq!(parsed.sum_num, 3);
    assert_eq!(parsed.items.len(), 3);
    assert!(parsed.items.iter().all(|i| {
        [
            "34020000001320000001",
            "34020000001320000004",
            "34020000001320000005",
        ]
        .contains(&i.device_id.as_str())
    }));
}

#[test]
fn catalog_empty_result_emits_single_message_with_sum_zero() {
    let provider = Arc::new(TestCatalogProvider { all: vec![] });
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let bodies = catalog_response_bodies(&outputs);
    assert_eq!(bodies.len(), 1);
    let parsed = parse_catalog(bodies[0].as_bytes()).unwrap();
    assert_eq!(parsed.sum_num, 0);
    assert_eq!(parsed.num, 0);
    assert!(parsed.items.is_empty());
}

#[test]
fn malformed_catalog_body_returns_bad_request() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let mut msg = catalog_query_message("1", "34020000001320000001", &call_id);
    if let SipMessage::Request { body, .. } = &mut msg {
        *body = b"not xml".to_vec();
    }
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
    assert_eq!(resp.body().len(), 0);
}

#[test]
fn non_catalog_query_returns_bad_request() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let body = br#"<?xml version="1.0"?>
<Query>
    <CmdType>DeviceInfo</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
</Query>"#;
    let mut msg = catalog_query_message("1", "34020000001320000001", &call_id);
    if let SipMessage::Request { body: req_body, .. } = &mut msg {
        *req_body = body.to_vec();
    }
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
    assert_eq!(resp.body().len(), 0);
}

#[test]
fn catalog_message_preserves_response_headers() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 2;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected first output to be SendResponse");
    };
    let cseq = resp.cseq().unwrap();
    assert_eq!(cseq.0, 1);
    assert_eq!(cseq.1, Method::Message);
    assert!(resp.call_id().is_some());
    assert!(resp.headers().get(&HeaderName::Via).is_some());
}

fn plain_message_request(call_id: &str) -> SipMessage {
    let body: Body = b"plain text body".to_vec();
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-abc").unwrap(),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&upstream_uri(), "upstream-tag").unwrap(),
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&local_uri()));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(1, Method::Message));
    headers.append(HeaderName::ContentType, HeaderValue::new("text/plain"));
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    SipMessage::Request {
        line: RequestLine::new(Method::Message, local_uri()),
        headers,
        body,
    }
}

#[test]
fn message_without_manscdp_content_type_returns_ok_and_no_catalog() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = plain_message_request(&call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    assert!(matches!(outputs[0], CascadeOutput::SendResponse(_)));
}

#[test]
fn catalog_query_respects_max_pages_cap() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 2;
    cfg.catalog_max_query_pages = 2;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let bodies = catalog_response_bodies(&outputs);
    assert_eq!(bodies.len(), 2);
    for body in &bodies {
        let parsed = parse_catalog(body.as_bytes()).unwrap();
        assert_eq!(parsed.sum_num, 4);
        assert_eq!(parsed.items.len(), 2);
    }
}

struct StuckCatalogProvider;

impl CatalogProvider for StuckCatalogProvider {
    fn query_page(
        &self,
        _query: &CatalogQuery,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<CatalogPage, CatalogError> {
        // Always returns the same two items and claims there are 100 total.
        Ok(CatalogPage {
            items: vec![
                CatalogItem {
                    device_id: "34020000001320000001".to_string(),
                    name: Some("Camera 1".to_string()),
                    status: Some("ON".to_string()),
                    ..Default::default()
                },
                CatalogItem {
                    device_id: "34020000001320000002".to_string(),
                    name: Some("Camera 2".to_string()),
                    status: Some("ON".to_string()),
                    ..Default::default()
                },
            ],
            total: 100,
            next_cursor: Some("stuck".to_string()),
        })
    }
}

struct FailingCatalogProvider;

impl CatalogProvider for FailingCatalogProvider {
    fn query_page(
        &self,
        _query: &CatalogQuery,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<CatalogPage, CatalogError> {
        Err(CatalogError::internal("database unavailable"))
    }
}

#[test]
fn catalog_provider_error_returns_500() {
    let provider = Arc::new(FailingCatalogProvider);
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
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
    assert!(resp.body().is_empty());
    let cseq = resp.cseq().unwrap();
    assert_eq!(cseq.0, 1);
    assert_eq!(cseq.1, Method::Message);
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 500),
        _ => panic!("expected a response"),
    }
}

#[test]
fn catalog_query_does_not_loop_forever_on_inconsistent_provider() {
    let provider = Arc::new(StuckCatalogProvider);
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 2;
    cfg.catalog_max_query_pages = 3;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let bodies = catalog_response_bodies(&outputs);
    assert_eq!(bodies.len(), 3);
    for body in &bodies {
        let parsed = parse_catalog(body.as_bytes()).unwrap();
        assert_eq!(parsed.sum_num, 6);
        assert_eq!(parsed.items.len(), 2);
    }
}

#[test]
fn catalog_query_when_unregistered_returns_403() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);

    let msg = catalog_query_message("1", "34020000001320000001", "ignored");
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
        _ => panic!("expected a response"),
    }
}

fn catalog_query_message_with_from(
    sn: &str,
    device_id: &str,
    from: SipUri,
    tag: &str,
    call_id: &str,
) -> SipMessage {
    let body = format!(
        r#"<?xml version="1.0"?>
<Query>
    <CmdType>Catalog</CmdType>
    <SN>{sn}</SN>
    <DeviceID>{device_id}</DeviceID>
</Query>"#
    );
    let body_bytes: Body = body.into_bytes();

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "upstream.example.com", 5060, "z9hG4bK-abc").unwrap(),
    );
    headers.append(HeaderName::From, HeaderValue::from_uri(&from, tag).unwrap());
    headers.append(HeaderName::To, HeaderValue::to_uri(&local_uri()));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 MESSAGE"));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body_bytes.len().to_string()),
    );

    SipMessage::Request {
        line: RequestLine::new(Method::Message, local_uri()),
        headers,
        body: body_bytes,
    }
}

#[test]
fn catalog_query_from_unknown_upstream_returns_403() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let foreign = SipUri::parse("sip:attacker@evil.example.com").unwrap();
    let msg =
        catalog_query_message_with_from("1", "34020000001320000001", foreign, "bad", &call_id);
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
        _ => panic!("expected a response"),
    }
}

#[test]
fn catalog_response_includes_local_to_tag() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert!(!outputs.is_empty());
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected SendResponse");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 200),
        _ => panic!("expected a response"),
    }
    let to = resp.headers().get(&HeaderName::To).unwrap();
    assert!(to.as_str().contains("tag="));
}

#[test]
fn catalog_query_with_new_call_id_succeeds() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let _ = register_to_connected(&mut cascade);

    // A real upstream platform uses a fresh Call-ID for each MESSAGE query.
    let msg = catalog_query_message("1", "34020000001320000001", "call-standalone-1");
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    assert!(!outputs.is_empty());
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected SendResponse");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 200),
        _ => panic!("expected a response"),
    }
}

#[test]
fn catalog_query_wrong_device_id_returns_bad_request() {
    let provider = setup_provider();
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let _ = register_to_connected(&mut cascade);

    // The queried DeviceID must equal the local platform ID.
    let msg = catalog_query_message("1", "34020000001320000002", "call-standalone-2");
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
        _ => panic!("expected a response"),
    }
}

struct HugeTotalCatalogProvider;

impl CatalogProvider for HugeTotalCatalogProvider {
    fn query_page(
        &self,
        _query: &CatalogQuery,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<CatalogPage, CatalogError> {
        Ok(CatalogPage {
            items: vec![CatalogItem {
                device_id: "34020000001320000001".to_string(),
                name: Some("Camera 1".to_string()),
                status: Some("ON".to_string()),
                ..Default::default()
            }],
            total: usize::MAX,
            next_cursor: None,
        })
    }
}

#[test]
fn catalog_query_clamps_huge_max_items_and_pages() {
    let provider = Arc::new(HugeTotalCatalogProvider);
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = u32::MAX;
    cfg.catalog_max_query_pages = u32::MAX;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let bodies = catalog_response_bodies(&outputs);
    assert_eq!(bodies.len(), 1);
    let parsed = parse_catalog(bodies[0].as_bytes()).unwrap();
    assert_eq!(parsed.items.len(), 1);
    // The advertised total is clamped to max_per_packet * max_pages (10_000 * 10_000).
    assert_eq!(parsed.sum_num, 100_000_000);
}

#[test]
fn catalog_error_clamps_internal_message() {
    let long = "x".repeat(2048);
    let err = CatalogError::internal(format!("provider failure: {long}"));
    let CatalogError::Internal(msg) = err;
    assert_eq!(msg.len(), 1024);
    assert!(msg.is_char_boundary(msg.len()));
    assert!(msg.starts_with("provider failure: "));
}

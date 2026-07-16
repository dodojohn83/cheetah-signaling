//! Security/ACL tests for upstream catalog sharing.

use std::sync::Arc;

use super::{config, local_uri, password_provider, register_to_connected};
use crate::cascade::{
    CascadeEvent, CascadeInput, CascadeOutput, CatalogError, CatalogPage, CatalogProvider,
    CatalogQuery, Gb28181Cascade,
};
use cheetah_gb28181_core::{
    Body, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage,
};

struct DummyProvider;

impl CatalogProvider for DummyProvider {
    fn query_page(
        &self,
        _query: &CatalogQuery,
        _offset: usize,
        _limit: usize,
    ) -> Result<CatalogPage, CatalogError> {
        Ok(CatalogPage {
            items: vec![],
            total: 0,
        })
    }
}

fn catalog_query_message_raw_from(
    sn: &str,
    device_id: &str,
    raw_from: &str,
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
    headers.append(HeaderName::From, HeaderValue::new(raw_from));
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
fn catalog_query_malformed_from_header_returns_403() {
    let provider = Arc::new(DummyProvider);
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider()).with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    // A display name containing '>' before the URI bracket must not panic and
    // the URI after the bracket must still be validated against the upstream.
    let raw_from = r#""a > b" <sip:attacker@evil.example.com>;tag=upstream-tag"#;
    let msg = catalog_query_message_raw_from("1", "34020000001320000001", raw_from, &call_id);
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

//! Security/ACL tests for upstream catalog sharing.

use std::sync::Arc;

use super::catalog::catalog_query_message;
use super::{config, local_uri, password_provider, register_to_connected, upstream_uri};
use crate::cascade::{
    CascadeConfig, CascadeEvent, CascadeInput, CascadeOutput, CatalogError, CatalogPage,
    CatalogProvider, CatalogQuery, Gb28181Cascade,
};
use cheetah_gb28181_core::{
    Body, DigestChallenge, DigestClient, HeaderName, HeaderValue, Method, RequestLine, SipHeaders,
    SipMessage,
};
use secrecy::SecretString;

struct DummyProvider;

impl CatalogProvider for DummyProvider {
    fn query_page(
        &self,
        _query: &CatalogQuery,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<CatalogPage, CatalogError> {
        Ok(CatalogPage {
            items: vec![],
            total: 0,
            next_cursor: None,
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

fn catalog_query_message_with_auth(
    sn: &str,
    device_id: &str,
    call_id: &str,
    auth_value: &str,
) -> SipMessage {
    let mut msg = catalog_query_message(sn, device_id, call_id);
    if let SipMessage::Request { headers, .. } = &mut msg {
        headers.append(HeaderName::Authorization, HeaderValue::new(auth_value));
    }
    msg
}

fn inbound_digest_config() -> CascadeConfig {
    config()
        .with_catalog_inbound_digest(
            "inbound",
            b"this-secret-is-much-longer-than-32-bytes-and-used-for-inbound-digest-tests",
        )
        .unwrap()
}

fn extract_www_authenticate(outputs: &[CascadeOutput]) -> Option<String> {
    outputs.iter().find_map(|o| match o {
        CascadeOutput::SendResponse(SipMessage::Response { headers, .. }) => headers
            .get(&HeaderName::WwwAuthenticate)
            .map(|v| v.as_str().to_string()),
        _ => None,
    })
}

#[test]
fn catalog_query_malformed_from_header_returns_403() {
    let provider = Arc::new(DummyProvider);
    let mut cfg = config();
    cfg.catalog_max_items_per_packet = 100;
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
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

#[test]
fn catalog_query_without_authorization_returns_401_challenge() {
    let provider = Arc::new(DummyProvider);
    let cfg = inbound_digest_config();
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
    match resp {
        SipMessage::Response { line, headers, .. } => {
            assert_eq!(line.code, 401);
            assert!(headers.get(&HeaderName::WwwAuthenticate).is_some());
        }
        _ => panic!("expected a response"),
    }
}

#[test]
fn catalog_query_with_valid_authorization_returns_200_and_catalog() {
    let provider = Arc::new(DummyProvider);
    let cfg = inbound_digest_config();
    let mut cascade = Gb28181Cascade::new(cfg, password_provider())
        .unwrap()
        .with_catalog_provider(provider);
    let (call_id, _) = register_to_connected(&mut cascade);

    // First request: get challenge.
    let msg = catalog_query_message("1", "34020000001320000001", &call_id);
    let outputs = cascade
        .process(CascadeInput {
            now: 100,
            event: CascadeEvent::Request(Box::new(msg)),
        })
        .unwrap();

    let Some(challenge_value) = extract_www_authenticate(&outputs) else {
        panic!("expected a WWW-Authenticate challenge");
    };
    let challenge = DigestChallenge::parse(&challenge_value).unwrap();

    // Build Authorization response using the same password as the provider.
    let password = SecretString::new("secret".into());
    let cnonce = DigestClient::derive_cnonce(&password, &call_id).unwrap();
    let mut client = DigestClient::new();
    let digest_response = client
        .authorize(
            upstream_uri().user().unwrap_or("upstream"),
            &password,
            "MESSAGE",
            &local_uri().encode(),
            &challenge,
            &cnonce,
        )
        .unwrap();

    let auth_msg = catalog_query_message_with_auth(
        "1",
        "34020000001320000001",
        &call_id,
        &digest_response.to_header_value(),
    );
    let outputs = cascade
        .process(CascadeInput {
            now: 101,
            event: CascadeEvent::Request(Box::new(auth_msg)),
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    let CascadeOutput::SendResponse(resp) = &outputs[0] else {
        panic!("expected SendResponse");
    };
    match resp {
        SipMessage::Response { line, .. } => assert_eq!(line.code, 200),
        _ => panic!("expected a response"),
    }
    assert!(matches!(outputs[1], CascadeOutput::SendRequest(_)));
}

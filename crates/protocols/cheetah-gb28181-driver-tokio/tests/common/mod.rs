//! Shared helpers for GB28181 driver integration tests.
#![allow(dead_code)]

use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipParser,
    SipParserConfig, SipUri, encode_message,
};
use cheetah_gb28181_module::{
    AuthPolicy, CredentialError, CredentialProvider, DeviceId, Gb28181Access, Gb28181DomainConfig,
};
use secrecy::SecretString;
use std::collections::HashMap;

/// Builds a test domain config with the supplied auth policy.
pub fn test_domain_config(policy: AuthPolicy) -> Gb28181DomainConfig {
    Gb28181DomainConfig::new(
        "test-domain",
        "test.realm",
        std::iter::repeat_n(b'a', 32).collect::<Vec<u8>>(),
    )
    .expect("valid test config")
    .with_auth_policy(policy)
}

/// Builds a closure credential provider backed by an in-memory password map.
pub fn test_credential_provider(
    passwords: HashMap<String, SecretString>,
) -> impl CredentialProvider + 'static {
    move |id: &DeviceId| -> Result<Option<SecretString>, CredentialError> {
        Ok(passwords.get(&id.to_string()).cloned())
    }
}

/// Builds an access machine requiring authentication with no configured
/// passwords, so every REGISTER receives a 401 challenge.
pub fn challenging_access() -> Gb28181Access<impl CredentialProvider + 'static> {
    let domain = test_domain_config(AuthPolicy::Required);
    let provider = test_credential_provider(HashMap::new());
    Gb28181Access::new(domain, provider).expect("valid access")
}

/// Builds a minimal REGISTER request for `device_id` advertising `port`.
pub fn build_register_request(device_id: &str, port: u16) -> SipMessage {
    let uri = SipUri::parse(format!("sip:{device_id}@127.0.0.1")).expect("valid uri");
    let from_uri = SipUri::parse(format!("sip:{device_id}@127.0.0.1")).expect("valid uri");
    let contact_uri =
        SipUri::parse(format!("sip:{device_id}@127.0.0.1:{port}")).expect("valid uri");

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", "127.0.0.1", port, &format!("z9hG4bK{device_id}"))
            .expect("valid via"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&from_uri, &format!("tag-{device_id}")).expect("valid from"),
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&from_uri));
    headers.append(
        HeaderName::CallId,
        HeaderValue::new(format!("call-{device_id}")),
    );
    headers.append(HeaderName::CSeq, HeaderValue::cseq(1, Method::Register));
    headers.append(HeaderName::Contact, HeaderValue::contact_uri(&contact_uri));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(HeaderName::Expires, HeaderValue::new("3600"));

    SipMessage::Request {
        line: RequestLine::new(Method::Register, uri),
        headers,
        body: Vec::new(),
    }
}

/// Encodes a REGISTER request to wire bytes with a Content-Length header.
pub fn encode_register(device_id: &str, port: u16) -> Vec<u8> {
    encode_message(&build_register_request(device_id, port))
}

/// Parses a single SIP datagram (used for UDP responses).
pub fn parse_datagram(bytes: &[u8]) -> SipMessage {
    SipParser::parse_datagram(bytes, SipParserConfig::default()).expect("parse response")
}

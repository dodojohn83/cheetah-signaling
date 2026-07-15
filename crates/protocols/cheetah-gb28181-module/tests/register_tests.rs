//! Integration tests for GB28181 REGISTER handling.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri,
};
use cheetah_gb28181_module::{
    AccessInput, AccessOutput, DeviceId, Gb28181Access, Gb28181DomainConfig,
};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

const REALM: &str = "example.com";
const DEVICE_ID: &str = "34020000001320000001";
const PASSWORD: &str = "secret";
const SERVER_SECRET: &[u8] = b"server-secret-must-be-32-bytes-long";

fn make_request(cseq: u32, with_auth: bool) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKabc"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-1"));
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{cseq} REGISTER")),
    );
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@192.168.1.100:5060>;expires=3600")),
    );
    headers.append(HeaderName::UserAgent, HeaderValue::new("IPC"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    if with_auth {
        // Authorization is added by the caller after receiving the challenge.
    }

    SipMessage::Request {
        line: RequestLine::new(
            Method::Register,
            SipUri::parse(format!("sip:{DEVICE_ID}@{REALM}")).unwrap(),
        ),
        headers,
        body: Vec::new(),
    }
}

fn add_authorization(request: &mut SipMessage, nonce: &str) {
    let response = compute_response(nonce);
    let value = format!(
        r##"Digest username="{DEVICE_ID}", realm="{REALM}", nonce="{nonce}", uri="sip:{DEVICE_ID}@{REALM}", response="{response}", cnonce="clientnonce", nc="00000001", qop="auth", algorithm="SHA-256""##
    );
    request
        .headers_mut()
        .append(HeaderName::Authorization, HeaderValue::new(value));
}

fn compute_response(nonce: &str) -> String {
    let a1 = format!("{DEVICE_ID}:{REALM}:{PASSWORD}");
    let ha1 = hash_hex(&a1);
    let a2 = format!("REGISTER:sip:{DEVICE_ID}@{REALM}");
    let ha2 = hash_hex(&a2);
    let a3 = format!("{ha1}:{nonce}:00000001:clientnonce:auth:{ha2}");
    hash_hex(&a3)
}

fn hash_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn extract_nonce(header: &str) -> String {
    header
        .split(',')
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix("nonce=\"")
                .and_then(|v| v.split('\"').next())
                .map(String::from)
        })
        .expect("nonce in challenge")
}

fn find_response(outputs: &[AccessOutput]) -> &SipMessage {
    outputs
        .iter()
        .find_map(|o| match o {
            AccessOutput::SendResponse(m) => Some(m),
            _ => None,
        })
        .expect("a response")
}

#[test]
fn unauthenticated_register_returns_401_challenge() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec()).unwrap();
    let provider = |device: &DeviceId| {
        if device.as_ref() == DEVICE_ID {
            Some(SecretString::from(PASSWORD))
        } else {
            None
        }
    };
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let request = make_request(1, false);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();

    assert_eq!(outputs.len(), 1);
    let response = find_response(&outputs);
    let SipMessage::Response { line, headers, .. } = response else {
        panic!("expected response");
    };
    assert_eq!(line.code, 401);
    let www_auth = headers
        .get(&HeaderName::WwwAuthenticate)
        .expect("WWW-Authenticate");
    assert!(www_auth.as_str().contains("Digest "));
    assert!(www_auth.as_str().contains("algorithm=SHA-256"));
}

#[test]
fn authenticated_register_returns_200_and_emits_event() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec()).unwrap();
    let provider = |device: &DeviceId| {
        if device.as_ref() == DEVICE_ID {
            Some(SecretString::from(PASSWORD))
        } else {
            None
        }
    };
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let mut request = make_request(1, false);
    let challenge_outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request.clone(),
        })
        .unwrap();
    let challenge_response = find_response(&challenge_outputs);
    let www_auth = challenge_response
        .headers()
        .get(&HeaderName::WwwAuthenticate)
        .expect("WWW-Authenticate")
        .as_str();
    let nonce = extract_nonce(www_auth);

    add_authorization(&mut request, &nonce);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    let SipMessage::Response { line, headers, .. } = find_response(&outputs) else {
        panic!("expected response");
    };
    assert_eq!(line.code, 200);
    let expires = headers.get(&HeaderName::Expires).expect("Expires");
    assert_eq!(expires.as_str(), "3600");

    let registered = outputs.iter().find_map(|o| match o {
        AccessOutput::EmitEvent(e) => Some(e),
        _ => None,
    });
    assert!(matches!(
        registered,
        Some(cheetah_gb28181_module::Gb28181Event::DeviceRegistered { .. })
    ));
}

//! Integration tests for GB28181 REGISTER and Keepalive handling.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_gb28181_core::{
    DigestContext, DigestQop, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage,
    SipUri,
};
use cheetah_gb28181_module::{
    AccessInput, AccessOutput, AuthPolicy, DeviceId, DevicePresence, Gb28181Access,
    Gb28181DomainConfig, Gb28181Event,
};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

const REALM: &str = "example.com";
const DEVICE_ID: &str = "34020000001320000001";
const PASSWORD: &str = "secret";
const SERVER_SECRET: &[u8] = b"server-secret-must-be-32-bytes-long";

fn make_request(cseq: u32, with_auth: bool) -> SipMessage {
    make_register_request_for_id(DEVICE_ID, cseq, with_auth, 3600)
}

fn make_register_request(cseq: u32, with_auth: bool, expires: u32) -> SipMessage {
    make_register_request_for_id(DEVICE_ID, cseq, with_auth, expires)
}

fn make_register_request_for_id(
    device_id: &str,
    cseq: u32,
    with_auth: bool,
    expires: u32,
) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKabc"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{device_id}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{device_id}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-1"));
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{cseq} REGISTER")),
    );
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!(
            "<sip:{device_id}@192.168.1.100:5060>;expires={expires}"
        )),
    );
    headers.append(HeaderName::UserAgent, HeaderValue::new("IPC"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    if with_auth {
        // Authorization is added by the caller after receiving the challenge.
    }

    SipMessage::Request {
        line: RequestLine::new(
            Method::Register,
            SipUri::parse(format!("sip:{device_id}@{REALM}")).unwrap(),
        ),
        headers,
        body: Vec::new(),
    }
}

fn make_message_request(body: &[u8]) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKdef"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>;tag=fromtag")),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<sip:{DEVICE_ID}@{REALM}>")),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-2"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 MESSAGE".to_string()));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml".to_string()),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    SipMessage::Request {
        line: RequestLine::new(
            Method::Message,
            SipUri::parse(format!("sip:{DEVICE_ID}@{REALM}")).unwrap(),
        ),
        headers,
        body: body.to_vec(),
    }
}

fn keepalive_body() -> Vec<u8> {
    br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Status>OK</Status>
</Notify>"#
        .to_vec()
}

fn make_registered_access() -> (
    Gb28181Access<impl Fn(&DeviceId) -> Option<SecretString>>,
    u64,
) {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| None;
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let request = make_request(1, false);
    access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();

    (access, 1000)
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

fn find_events(outputs: &[AccessOutput]) -> impl Iterator<Item = &Gb28181Event> + '_ {
    outputs.iter().filter_map(|o| match o {
        AccessOutput::EmitEvent(e) => Some(e),
        _ => None,
    })
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
    let SipMessage::Response { line, .. } = response else {
        panic!("expected response");
    };
    assert_eq!(line.code, 401);
    assert!(
        response
            .headers()
            .get(&HeaderName::WwwAuthenticate)
            .is_some()
    );
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
    let response = find_response(&outputs);
    let SipMessage::Response { line, .. } = response else {
        panic!("expected response");
    };
    assert_eq!(line.code, 200);

    let registered = find_events(&outputs).find_map(|e| match e {
        Gb28181Event::DeviceRegistered { .. } => Some(e.clone()),
        _ => None,
    });
    assert!(matches!(
        registered,
        Some(Gb28181Event::DeviceRegistered { .. })
    ));
}

#[test]
fn challenge_optional_register_without_auth_succeeds() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| -> Option<SecretString> { None };
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let request = make_request(1, false);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    let response = find_response(&outputs);
    let SipMessage::Response { line, .. } = response else {
        panic!("expected response");
    };
    assert_eq!(line.code, 200);
}

#[test]
fn multiple_via_headers_are_copied_to_response() {
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
    request.headers_mut().append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bKproxy"),
    );

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

    let response = find_response(&outputs);
    let via_values: Vec<_> = response
        .headers()
        .get_all(&HeaderName::Via)
        .map(|v| v.as_str().to_string())
        .collect();
    assert_eq!(via_values.len(), 2);
    assert!(via_values.iter().any(|v| v.contains("192.168.1.100")));
    assert!(via_values.iter().any(|v| v.contains("10.0.0.1")));
}

#[test]
fn challenge_optional_accepts_valid_credentials() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |device: &DeviceId| {
        if device.as_ref() == DEVICE_ID {
            Some(SecretString::from(PASSWORD))
        } else {
            None
        }
    };
    let mut access = Gb28181Access::new(config, provider).unwrap();

    // ChallengeOptional mode accepts unauthenticated requests, so we build a
    // request that already carries valid credentials to test the validation path.
    let ctx = DigestContext::new(REALM, SERVER_SECRET)
        .unwrap()
        .qop(Some(DigestQop::Auth))
        .unwrap();
    let nonce = ctx.generate_challenge(1000).unwrap().nonce;

    let mut request = make_request(1, false);
    add_authorization(&mut request, &nonce);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();

    assert_eq!(outputs.len(), 2);
    let response = find_response(&outputs);
    let SipMessage::Response { line, .. } = response else {
        panic!("expected response");
    };
    assert_eq!(line.code, 200);
}

#[test]
fn keepalive_before_register_is_rejected() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| None;
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let request = make_message_request(&keepalive_body());
    let result = access.process(AccessInput {
        source: "192.168.1.100:5060".parse().unwrap(),
        now: 1000,
        message: request,
    });

    assert!(matches!(
        result,
        Err(cheetah_gb28181_module::AccessError::NotRegistered)
    ));
}

#[test]
fn keepalive_after_register_succeeds() {
    let (mut access, now) = make_registered_access();

    let request = make_message_request(&keepalive_body());
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now,
            message: request,
        })
        .unwrap();

    let response = find_response(&outputs);
    let SipMessage::Response { line, .. } = response else {
        panic!("expected response");
    };
    assert_eq!(line.code, 200);

    let mut keepalive_seen = false;
    for event in find_events(&outputs) {
        match event {
            Gb28181Event::Keepalive {
                device_id, status, ..
            } => {
                assert_eq!(device_id.as_ref(), DEVICE_ID);
                assert_eq!(status, "OK");
                keepalive_seen = true;
            }
            Gb28181Event::DevicePresenceChanged { .. } => {
                panic!("unexpected presence change on first keepalive");
            }
            _ => {}
        }
    }
    assert!(keepalive_seen);
}

#[test]
fn heartbeat_timeout_emits_offline_event() {
    let (mut access, now) = make_registered_access();
    let heartbeat_timeout = 90;

    let offline_outputs = access.tick(now + heartbeat_timeout + 1);
    assert_eq!(offline_outputs.len(), 1);
    let event = find_events(&offline_outputs).next().expect("an event");
    match event {
        Gb28181Event::DevicePresenceChanged {
            presence: DevicePresence::Offline,
            ..
        } => {}
        _ => panic!("expected offline presence change, got {event:?}"),
    }
}

#[test]
fn keepalive_after_offline_restores_online() {
    let (mut access, now) = make_registered_access();
    let heartbeat_timeout = 90;

    let _offline_outputs = access.tick(now + heartbeat_timeout + 1);

    let request = make_message_request(&keepalive_body());
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: now + heartbeat_timeout + 2,
            message: request,
        })
        .unwrap();

    let mut online_seen = false;
    let mut keepalive_seen = false;
    for event in find_events(&outputs) {
        match event {
            Gb28181Event::DevicePresenceChanged {
                presence: DevicePresence::Online,
                ..
            } => {
                online_seen = true;
            }
            Gb28181Event::Keepalive { .. } => keepalive_seen = true,
            _ => {}
        }
    }
    assert!(online_seen);
    assert!(keepalive_seen);
}

#[test]
fn registration_expiry_removes_registration() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| None;
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let request = make_register_request(1, false, 5);
    access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();

    let expired_outputs = access.tick(1006);
    assert_eq!(expired_outputs.len(), 1);
    let event = find_events(&expired_outputs).next().expect("an event");
    assert!(matches!(event, Gb28181Event::DeviceUnregistered { .. }));

    // A keepalive after expiry should be rejected because the device is no
    // longer registered.
    let request = make_message_request(&keepalive_body());
    let result = access.process(AccessInput {
        source: "192.168.1.100:5060".parse().unwrap(),
        now: 1006,
        message: request,
    });
    assert!(matches!(
        result,
        Err(cheetah_gb28181_module::AccessError::NotRegistered)
    ));
}

#[test]
fn registration_table_respects_capacity_limit() {
    let config =
        Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
            .unwrap()
            .with_auth_policy(AuthPolicy::ChallengeOptional)
            .with_max_registrations(1);
    let provider = |_device: &DeviceId| None;
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let request = make_request(1, false);
    access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();

    // A second device with a different device ID should be rejected because the
    // table is at capacity.
    let other_request =
        make_register_request_for_id("34020000001320000002", 1, false, 3600);

    let result = access.process(AccessInput {
        source: "192.168.1.101:5060".parse().unwrap(),
        now: 1000,
        message: other_request,
    });
    assert!(matches!(
        result,
        Err(cheetah_gb28181_module::AccessError::RegistrationTableFull)
    ));
}

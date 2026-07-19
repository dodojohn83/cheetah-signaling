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
fn challenge_optional_ignores_unknown_cached_credentials() {
    // Devices may replay stale Authorization headers from a previous session.
    // In ChallengeOptional mode, an unknown device presenting such a header
    // must still be accepted rather than rejected with AuthenticationFailed.
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| -> Option<SecretString> { None };
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let mut request = make_request(1, false);
    request.headers_mut().append(
        HeaderName::Authorization,
        HeaderValue::new("Digest username=\"34020000001320000001\", realm=\"example.com\", nonce=\"deadbeef\", uri=\"sip:34020000001320000001@example.com\", response=\"fakemac\", algorithm=\"SHA-256\""),
    );
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
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
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
    let other_request = make_register_request_for_id("34020000001320000002", 1, false, 3600);

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

#[test]
fn catalog_message_emits_catalog_received_event() {
    let (mut access, now) = make_registered_access();
    let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>Catalog</CmdType>
    <SN>2</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <SumNum>1</SumNum>
    <DeviceList Num="1">
        <Item>
            <DeviceID>34020000001320000001</DeviceID>
            <Name>Camera 1</Name>
            <Status>ON</Status>
        </Item>
    </DeviceList>
</Response>"#;
    let request = make_message_request(body);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: now + 1,
            message: request,
        })
        .unwrap();

    let mut catalog_seen = false;
    let mut response_seen = false;
    for output in outputs {
        match output {
            AccessOutput::EmitEvent(Gb28181Event::CatalogReceived {
                sn,
                sum_num,
                num,
                items,
                ..
            }) => {
                assert_eq!(sn, "2");
                assert_eq!(sum_num, 1);
                assert_eq!(num, 1);
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].device_id, "34020000001320000001");
                assert_eq!(items[0].name.as_deref(), Some("Camera 1"));
                catalog_seen = true;
            }
            AccessOutput::SendResponse(_) => response_seen = true,
            _ => {}
        }
    }
    assert!(catalog_seen);
    assert!(response_seen);
}

#[test]
fn device_info_message_emits_device_info_received_event() {
    let (mut access, now) = make_registered_access();
    let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>DeviceInfo</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>OK</Result>
    <Manufacturer>Hikvision</Manufacturer>
    <Model>DS-2CD</Model>
    <Firmware>V5.5.0</Firmware>
</Response>"#;
    let request = make_message_request(body);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: now + 1,
            message: request,
        })
        .unwrap();

    let mut seen = false;
    for output in outputs {
        if let AccessOutput::EmitEvent(Gb28181Event::DeviceInfoReceived {
            sn,
            result,
            manufacturer,
            model,
            firmware,
            ..
        }) = output
        {
            assert_eq!(sn, "1");
            assert_eq!(result.as_deref(), Some("OK"));
            assert_eq!(manufacturer.as_deref(), Some("Hikvision"));
            assert_eq!(model.as_deref(), Some("DS-2CD"));
            assert_eq!(firmware.as_deref(), Some("V5.5.0"));
            seen = true;
        }
    }
    assert!(seen);
}

#[test]
fn device_status_message_emits_device_status_received_event() {
    let (mut access, now) = make_registered_access();
    let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>DeviceStatus</CmdType>
    <SN>3</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>OK</Result>
    <Online>ONLINE</Online>
    <Status>OK</Status>
    <InvalidEquip>False</InvalidEquip>
</Response>"#;
    let request = make_message_request(body);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: now + 1,
            message: request,
        })
        .unwrap();

    let mut seen = false;
    for output in outputs {
        if let AccessOutput::EmitEvent(Gb28181Event::DeviceStatusReceived {
            sn,
            result,
            online,
            status,
            invalid_equip,
            ..
        }) = output
        {
            assert_eq!(sn, "3");
            assert_eq!(result.as_deref(), Some("OK"));
            assert_eq!(online.as_deref(), Some("ONLINE"));
            assert_eq!(status.as_deref(), Some("OK"));
            assert_eq!(invalid_equip.as_deref(), Some("False"));
            seen = true;
        }
    }
    assert!(seen);
}

#[test]
fn unregistered_device_message_is_rejected() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| None;
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Status>OK</Status>
</Notify>"#;
    let request = make_message_request(body);
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
fn mismatched_xml_device_id_is_rejected() {
    let (mut access, now) = make_registered_access();
    let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000002</DeviceID>
    <Status>OK</Status>
</Notify>"#;
    let request = make_message_request(body);
    let result = access.process(AccessInput {
        source: "192.168.1.100:5060".parse().unwrap(),
        now: now + 1,
        message: request,
    });
    assert!(matches!(
        result,
        Err(cheetah_gb28181_module::AccessError::InvalidDeviceId)
    ));
}

#[test]
fn alarm_message_emits_alarm_received_event() {
    let (mut access, now) = make_registered_access();
    let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Alarm</CmdType>
    <SN>5</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <AlarmPriority>1</AlarmPriority>
    <AlarmMethod>2</AlarmMethod>
    <AlarmType>1</AlarmType>
    <AlarmTime>2026-07-13T14:31:00</AlarmTime>
    <Info>motion</Info>
</Notify>"#;
    let request = make_message_request(body);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: now + 1,
            message: request,
        })
        .unwrap();

    let mut seen = false;
    for output in outputs {
        if let AccessOutput::EmitEvent(Gb28181Event::AlarmReceived {
            sn,
            priority,
            method,
            alarm_type,
            time,
            info,
            ..
        }) = output
        {
            assert_eq!(sn, "5");
            assert_eq!(priority.as_deref(), Some("1"));
            assert_eq!(method.as_deref(), Some("2"));
            assert_eq!(alarm_type.as_deref(), Some("1"));
            assert_eq!(time.as_deref(), Some("2026-07-13T14:31:00"));
            assert_eq!(info.as_deref(), Some("motion"));
            seen = true;
        }
    }
    assert!(seen);
}

#[test]
fn mobile_position_message_emits_mobile_position_received_event() {
    let (mut access, now) = make_registered_access();
    let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>MobilePosition</CmdType>
    <SN>6</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Time>2026-07-13T14:31:00</Time>
    <Longitude>121.47</Longitude>
    <Latitude>31.23</Latitude>
    <Speed>60.5</Speed>
    <Direction>180</Direction>
    <Altitude>10</Altitude>
</Notify>"#;
    let request = make_message_request(body);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: now + 1,
            message: request,
        })
        .unwrap();

    let mut seen = false;
    for output in outputs {
        if let AccessOutput::EmitEvent(Gb28181Event::MobilePositionReceived {
            sn,
            time,
            longitude,
            latitude,
            speed,
            direction,
            altitude,
            ..
        }) = output
        {
            assert_eq!(sn, "6");
            assert_eq!(time.as_deref(), Some("2026-07-13T14:31:00"));
            assert_eq!(longitude.as_deref(), Some("121.47"));
            assert_eq!(latitude.as_deref(), Some("31.23"));
            assert_eq!(speed.as_deref(), Some("60.5"));
            assert_eq!(direction.as_deref(), Some("180"));
            assert_eq!(altitude.as_deref(), Some("10"));
            seen = true;
        }
    }
    assert!(seen);
}

#[test]
fn record_info_message_emits_record_info_received_event() {
    let (mut access, now) = make_registered_access();
    let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>RecordInfo</CmdType>
    <SN>7</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Name>Camera 1</Name>
    <SumNum>1</SumNum>
    <RecordList Num="1">
        <Item>
            <DeviceID>34020000001320000001</DeviceID>
            <Name>Channel 1</Name>
            <FilePath>/path/to/file</FilePath>
            <StartTime>2026-07-13T00:00:00</StartTime>
            <EndTime>2026-07-13T01:00:00</EndTime>
            <Secrecy>0</Secrecy>
            <Type>time</Type>
            <FileSize>1024</FileSize>
        </Item>
    </RecordList>
</Response>"#;
    let request = make_message_request(body);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: now + 1,
            message: request,
        })
        .unwrap();

    let mut seen = false;
    for output in outputs {
        if let AccessOutput::EmitEvent(Gb28181Event::RecordInfoReceived {
            sn,
            name,
            sum_num,
            num,
            items,
            ..
        }) = output
        {
            assert_eq!(sn, "7");
            assert_eq!(name.as_deref(), Some("Camera 1"));
            assert_eq!(sum_num, 1);
            assert_eq!(num, 1);
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].device_id, "34020000001320000001");
            assert_eq!(items[0].file_path.as_deref(), Some("/path/to/file"));
            assert_eq!(items[0].file_size.as_deref(), Some("1024"));
            seen = true;
        }
    }
    assert!(seen);
}

#[test]
fn device_control_response_emits_device_control_response_received_event() {
    let (mut access, now) = make_registered_access();
    let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>DeviceControl</CmdType>
    <SN>42</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>OK</Result>
</Response>"#;
    let request = make_message_request(body);
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: now + 1,
            message: request,
        })
        .unwrap();

    let mut seen = false;
    for output in outputs {
        if let AccessOutput::EmitEvent(Gb28181Event::DeviceControlResponseReceived {
            sn,
            result,
            ..
        }) = output
        {
            assert_eq!(sn, "42");
            assert_eq!(result.as_deref(), Some("OK"));
            seen = true;
        }
    }
    assert!(seen);
}

#[test]
fn register_rejects_malformed_expires_header() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| None;
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let mut request = make_request(1, false);
    request
        .headers_mut()
        .append(HeaderName::Expires, HeaderValue::new("not-a-number"));
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(matches!(
        &outputs[0],
        AccessOutput::SendResponse(SipMessage::Response { line, .. }) if line.code == 400
    ));
}

#[test]
fn register_rejects_malformed_contact_expires_param() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| None;
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let base = make_request(1, false);
    let SipMessage::Request {
        line,
        headers,
        body,
    } = base
    else {
        panic!("expected request");
    };
    let mut new_headers = SipHeaders::new();
    for (name, value) in headers.iter() {
        if name != &HeaderName::Contact {
            new_headers.append(name.clone(), value.clone());
        }
    }
    new_headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!(
            "<sip:{DEVICE_ID}@192.168.1.100:5060>;expires=not-a-number"
        )),
    );
    let request = SipMessage::Request {
        line,
        headers: new_headers,
        body,
    };
    let outputs = access
        .process(AccessInput {
            source: "192.168.1.100:5060".parse().unwrap(),
            now: 1000,
            message: request,
        })
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(matches!(
        &outputs[0],
        AccessOutput::SendResponse(SipMessage::Response { line, .. }) if line.code == 400
    ));
}

#[test]
fn register_rejects_missing_device_id_with_400() {
    let config = Gb28181DomainConfig::new("domain-1", REALM, SERVER_SECRET.to_vec())
        .unwrap()
        .with_auth_policy(AuthPolicy::ChallengeOptional);
    let provider = |_device: &DeviceId| None;
    let mut access = Gb28181Access::new(config, provider).unwrap();

    let request = make_register_request_without_device_id(1, 3600);
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
    assert_eq!(line.code, 400);
}

#[test]
fn register_required_rejects_malformed_authorization_with_400() {
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
        HeaderName::Authorization,
        HeaderValue::new("Basic dXNlcjpwYXNz"),
    );
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
    assert_eq!(line.code, 400);
}

#[test]
fn register_required_rejects_invalid_credentials_with_401() {
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
        HeaderName::Authorization,
        HeaderValue::new(format!(
            "username=\"{}\", realm=\"{}\", nonce=\"deadbeef\", uri=\"sip:{}@{}\", response=\"0000000000000000000000000000000000000000000000000000000000000000\", algorithm=\"SHA-256\"",
            DEVICE_ID, REALM, DEVICE_ID, REALM
        )),
    );
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
    assert!(headers.get(&HeaderName::WwwAuthenticate).is_some());
}

fn make_register_request_without_device_id(cseq: u32, expires: u32) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKabc"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:example.com>;tag=fromtag"),
    );
    headers.append(HeaderName::To, HeaderValue::new("<sip:example.com>"));
    headers.append(HeaderName::CallId, HeaderValue::new("call-id-no-device"));
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{cseq} REGISTER")),
    );
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!(
            "<sip:{DEVICE_ID}@192.168.1.100:5060>;expires={expires}"
        )),
    );
    headers.append(HeaderName::UserAgent, HeaderValue::new("IPC"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    SipMessage::Request {
        line: RequestLine::new(Method::Register, SipUri::parse("sip:example.com").unwrap()),
        headers,
        body: Vec::new(),
    }
}

//! Tests for upstream event reporting (GB-CAS-005).

use crate::cascade::tests::{config, password_provider, register_to_connected};
use crate::cascade::{CascadeEvent, CascadeInput, CascadeOutput, Gb28181Cascade};
use crate::events::{DevicePresence, Gb28181Event};
use cheetah_gb28181_core::{HeaderName, Method, SipMessage};
use std::net::SocketAddr;

fn addr() -> SocketAddr {
    "127.0.0.1:5060".parse().unwrap()
}

fn presence_event(online: bool) -> Gb28181Event {
    Gb28181Event::DevicePresenceChanged {
        domain_id: crate::cascade::tests::domain_id(),
        device_id: crate::types::DeviceId::new("34020000001320000001").unwrap(),
        source: addr(),
        presence: if online {
            DevicePresence::Online
        } else {
            DevicePresence::Offline
        },
    }
}

fn alarm_event() -> Gb28181Event {
    Gb28181Event::AlarmReceived {
        domain_id: crate::cascade::tests::domain_id(),
        device_id: crate::types::DeviceId::new("34020000001320000001").unwrap(),
        source: addr(),
        sn: "42".to_string(),
        priority: Some("1".to_string()),
        method: Some("2".to_string()),
        alarm_type: Some("1".to_string()),
        time: Some("2026-07-13T14:31:00".to_string()),
        info: Some("motion".to_string()),
    }
}

fn mobile_position_event() -> Gb28181Event {
    Gb28181Event::MobilePositionReceived {
        domain_id: crate::cascade::tests::domain_id(),
        device_id: crate::types::DeviceId::new("34020000001320000001").unwrap(),
        source: addr(),
        sn: "7".to_string(),
        time: Some("2026-07-13T14:31:00".to_string()),
        longitude: Some("121.47".to_string()),
        latitude: Some("31.23".to_string()),
        speed: Some("60.5".to_string()),
        direction: Some("180".to_string()),
        altitude: Some("10".to_string()),
    }
}

#[test]
fn presence_report_is_held_until_registered_then_flushed_on_tick() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    let event = presence_event(true);
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Report {
                event: Box::new(event),
            },
        })
        .unwrap();
    assert!(outputs.is_empty());

    register_to_connected(&mut cascade);
    let outputs = cascade
        .process(CascadeInput {
            now: 1002,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    let requests: Vec<&SipMessage> = outputs
        .iter()
        .filter_map(|o| match o {
            CascadeOutput::SendRequest(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1);
    let SipMessage::Request { line, .. } = requests[0] else {
        panic!("expected request");
    };
    assert_eq!(line.method, Method::Message);
    let body = std::str::from_utf8(requests[0].body()).unwrap();
    assert!(body.contains("<CmdType>DeviceStatus</CmdType>"));
    assert!(body.contains("<Online>ONLINE</Online>"));
}

#[test]
fn state_events_merge_to_latest_snapshot() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    let online = presence_event(true);
    let offline = presence_event(false);
    cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Report {
                event: Box::new(online),
            },
        })
        .unwrap();
    cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Report {
                event: Box::new(offline),
            },
        })
        .unwrap();

    register_to_connected(&mut cascade);
    let outputs = cascade
        .process(CascadeInput {
            now: 1002,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    let body = outputs
        .iter()
        .find_map(|o| match o {
            CascadeOutput::SendRequest(m) => {
                Some(std::str::from_utf8(m.body()).unwrap().to_string())
            }
            _ => None,
        })
        .unwrap();
    assert!(body.contains("<Online>OFFLINE</Online>"));
    assert!(!body.contains("<Online>ONLINE</Online>"));
}

#[test]
fn alarm_report_carries_idempotency_key() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    let outputs = cascade
        .process(CascadeInput {
            now: 1002,
            event: CascadeEvent::Report {
                event: Box::new(alarm_event()),
            },
        })
        .unwrap();
    // Flushing happens on the next tick.
    assert!(outputs.is_empty());

    let outputs = cascade
        .process(CascadeInput {
            now: 1003,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    let request = outputs
        .iter()
        .find_map(|o| match o {
            CascadeOutput::SendRequest(m) => Some(m),
            _ => None,
        })
        .unwrap();
    let body = std::str::from_utf8(request.body()).unwrap();
    assert!(body.contains("<CmdType>Alarm</CmdType>"));
    let headers = request.headers();
    let key = headers
        .get(&HeaderName::Other("X-Idempotency-Key".to_string()))
        .unwrap();
    assert!(!key.as_str().is_empty());
}

#[test]
fn mobile_position_report_is_encoded() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    register_to_connected(&mut cascade);

    cascade
        .process(CascadeInput {
            now: 1002,
            event: CascadeEvent::Report {
                event: Box::new(mobile_position_event()),
            },
        })
        .unwrap();
    let outputs = cascade
        .process(CascadeInput {
            now: 1003,
            event: CascadeEvent::Tick,
        })
        .unwrap();

    let body = outputs
        .iter()
        .find_map(|o| match o {
            CascadeOutput::SendRequest(m) => {
                Some(std::str::from_utf8(m.body()).unwrap().to_string())
            }
            _ => None,
        })
        .unwrap();
    assert!(body.contains("<CmdType>MobilePosition</CmdType>"));
    assert!(body.contains("<Longitude>121.47</Longitude>"));
    assert!(body.contains("<Speed>60.5</Speed>"));
}

#[test]
fn unknown_event_is_silently_ignored() {
    let mut cascade = Gb28181Cascade::new(config(), password_provider()).unwrap();
    let event = Gb28181Event::Keepalive {
        domain_id: crate::cascade::tests::domain_id(),
        device_id: crate::types::DeviceId::new("34020000001320000001").unwrap(),
        source: addr(),
        status: "OK".to_string(),
    };
    let outputs = cascade
        .process(CascadeInput {
            now: 1000,
            event: CascadeEvent::Report {
                event: Box::new(event),
            },
        })
        .unwrap();
    assert!(outputs.is_empty());
}

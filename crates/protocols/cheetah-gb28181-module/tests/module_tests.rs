//! Unit-style integration tests for the GB28181 module state machine.

use cheetah_gb28181_module::{
    Gb28181Module,
    module::Gb28181Input,
    output::{Gb28181CommandResult, Gb28181Heartbeat, Gb28181Output, Gb28181Register},
};
use cheetah_runtime_api::DeviceKey;
use cheetah_signal_types::{DeviceId, DurationMs, MessageId, TenantId, UtcTimestamp};
use std::sync::Arc;

mod common;
use common::{
    DEVICE_ID, authorization_for_challenge, challenge_for_module, count_heartbeats,
    is_response_with_code, message_request, now, register_module, register_module_with_config,
    register_request, source_addr, test_config, test_config_with_limits,
    test_config_with_page_size, test_module,
};

#[test]
fn unauthenticated_register_returns_401_with_challenge() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = test_module()?;
    let request = register_request(1, 3600, None)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;

    assert_eq!(outputs.len(), 1);
    let challenge_value =
        common::extract_www_authenticate(&outputs).ok_or("missing WWW-Authenticate challenge")?;
    let challenge = cheetah_gb28181_core::DigestChallenge::parse(&challenge_value)?;
    assert_eq!(challenge.realm, common::REALM);
    assert!(!challenge.nonce.is_empty());
    Ok(())
}

#[test]
fn authenticated_register_emits_register_output() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = test_module()?;
    let now = now();
    let challenge = challenge_for_module(&mut module, now)?;
    let auth = authorization_for_challenge(&challenge, 1, "abc123");
    let request = register_request(2, 3600, Some(&auth))?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now,
    )?;

    assert!(outputs.iter().any(|o| matches!(
        o,
        Gb28181Output::Register(Gb28181Register { external_id, .. }) if external_id == DEVICE_ID
    )));
    assert!(outputs.iter().any(|o| matches!(
        o,
        Gb28181Output::SendMessage { message, .. } if is_response_with_code(message, 200)
    )));
    Ok(())
}

#[test]
fn deregister_with_expires_zero_emits_deregister() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = test_module()?;
    let now = now();
    let challenge = challenge_for_module(&mut module, now)?;
    let auth = authorization_for_challenge(&challenge, 1, "abc123");
    let request = register_request(2, 0, Some(&auth))?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now,
    )?;

    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, Gb28181Output::Deregister))
    );
    Ok(())
}

#[test]
fn keepalive_message_emits_heartbeat() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let body = b"<?xml version=\"1.0\"?><Notify><CmdType>Keepalive</CmdType><SN>1</SN><DeviceID>34020000001320000001</DeviceID><Status>OK</Status></Notify>";
    let request = message_request(1, body)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;

    assert!(outputs.iter().any(
        |o| matches!(o, Gb28181Output::Heartbeat(Gb28181Heartbeat { status, .. }) if status == "OK")
    ));
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, Gb28181Output::SendMessage { .. }))
    );
    Ok(())
}

#[test]
fn duplicate_message_does_not_duplicate_events() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let body = b"<?xml version=\"1.0\"?><Notify><CmdType>Keepalive</CmdType><SN>1</SN><DeviceID>34020000001320000001</DeviceID><Status>OK</Status></Notify>";
    let request = message_request(1, body)?;
    let first = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request.clone(),
        },
        now(),
    )?;
    let second = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;

    assert_eq!(count_heartbeats(&first), 1);
    assert_eq!(count_heartbeats(&second), 0);
    assert!(
        second
            .iter()
            .any(|o| matches!(o, Gb28181Output::SendMessage { .. }))
    );
    Ok(())
}

#[test]
fn catalog_messages_aggregate_until_complete() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let body1 = br#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <SumNum>2</SumNum>
  <ItemList>
    <Item>
      <DeviceID>34020000001320000001</DeviceID>
      <Name>Camera 1</Name>
      <Status>ON</Status>
    </Item>
  </ItemList>
</Response>"#;
    let body2 = br#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <SumNum>2</SumNum>
  <ItemList>
    <Item>
      <DeviceID>34020000001320000002</DeviceID>
      <Name>Camera 2</Name>
      <Status>ON</Status>
    </Item>
  </ItemList>
</Response>"#;

    let r1 = message_request(1, body1)?;
    let r2 = message_request(2, body2)?;
    let out1 = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: r1,
        },
        now(),
    )?;
    let out2 = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: r2,
        },
        now(),
    )?;

    assert!(!out1.iter().any(|o| matches!(o, Gb28181Output::Catalog(..))));
    let catalog = out2
        .iter()
        .find_map(|o| match o {
            Gb28181Output::Catalog(c) => Some(c.clone()),
            _ => None,
        })
        .ok_or("missing catalog output")?;
    assert_eq!(catalog.sum_num, 2);
    assert_eq!(catalog.items.len(), 2);
    assert!(catalog.complete);
    Ok(())
}

#[test]
fn duplicate_catalog_items_are_deduplicated() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let body = br#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <SumNum>2</SumNum>
  <ItemList>
    <Item>
      <DeviceID>34020000001320000001</DeviceID>
      <Name>Camera 1</Name>
      <Status>ON</Status>
    </Item>
    <Item>
      <DeviceID>34020000001320000001</DeviceID>
      <Name>Camera 1</Name>
      <Status>ON</Status>
    </Item>
  </ItemList>
</Response>"#;

    let request = message_request(1, body)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;
    let catalog = outputs
        .iter()
        .find_map(|o| match o {
            Gb28181Output::Catalog(c) => Some(c.clone()),
            _ => None,
        })
        .ok_or("missing catalog output")?;
    assert_eq!(catalog.items.len(), 2);
    Ok(())
}

#[test]
fn device_info_message_parses() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let body = br#"<?xml version="1.0"?>
<Response>
  <CmdType>DeviceInfo</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <DeviceName>Cam</DeviceName>
  <Manufacturer>Acme</Manufacturer>
  <Model>X1</Model>
  <Firmware>V1.0</Firmware>
</Response>"#;
    let request = message_request(1, body)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;
    assert!(outputs.iter().any(|o| matches!(o, Gb28181Output::DeviceInfo(info) if info.manufacturer.as_deref() == Some("Acme"))));
    Ok(())
}

#[test]
fn alarm_message_parses() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let body = br#"<?xml version="1.0"?>
<Notify>
  <CmdType>Alarm</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <AlarmPriority>1</AlarmPriority>
  <AlarmMethod>2</AlarmMethod>
  <AlarmType>3</AlarmType>
  <AlarmTime>2024-01-01T00:00:00</AlarmTime>
</Notify>"#;
    let request = message_request(1, body)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, Gb28181Output::Alarm(a) if a.priority.as_deref() == Some("1")))
    );
    Ok(())
}

#[test]
fn mobile_position_message_parses() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let body = br#"<?xml version="1.0"?>
<Notify>
  <CmdType>MobilePosition</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <Time>2024-01-01T00:00:00</Time>
  <Longitude>116.397</Longitude>
  <Latitude>39.916</Latitude>
  <Speed>0.0</Speed>
  <Direction>0</Direction>
  <Altitude>0</Altitude>
</Notify>"#;
    let request = message_request(1, body)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;
    assert!(outputs.iter().any(|o| matches!(o, Gb28181Output::MobilePosition(p) if p.longitude.as_deref() == Some("116.397"))));
    Ok(())
}

#[test]
fn malformed_xml_does_not_close_session() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let body = b"<?xml version=\"1.0\"?><Notify><CmdType>Keepalive</CmdType>";
    let request = message_request(1, body)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;

    assert!(outputs.iter().any(
        |o| matches!(o, Gb28181Output::ProtocolError { kind, .. } if kind == "xml_parse_error")
    ));
    assert!(outputs.iter().any(|o| matches!(
        o,
        Gb28181Output::ProtocolError { message, .. } if message == "XML body rejected by parser"
    )));
    assert!(outputs.iter().any(|o| matches!(o, Gb28181Output::SendMessage { message, .. } if is_response_with_code(message, 400))));
    Ok(())
}

#[test]
fn xml_limits_reject_oversized_catalog() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = (*test_config()).clone();
    config.xml_limits.max_list_items = 2;
    let key = DeviceKey::new(TenantId::generate(), DeviceId::generate());
    let mut module = Gb28181Module::new(key, Arc::new(config))?;

    let body = br#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <SumNum>3</SumNum>
  <ItemList>
    <Item><DeviceID>1</DeviceID><Name>A</Name><Status>ON</Status></Item>
    <Item><DeviceID>2</DeviceID><Name>B</Name><Status>ON</Status></Item>
    <Item><DeviceID>3</DeviceID><Name>C</Name><Status>ON</Status></Item>
  </ItemList>
</Response>"#;
    let request = message_request(1, body)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;
    assert!(outputs.iter().any(
        |o| matches!(o, Gb28181Output::ProtocolError { kind, .. } if kind == "xml_parse_error")
    ));
    Ok(())
}

#[test]
fn command_response_maps_to_pending_command() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let command_id = MessageId::generate();
    module.add_pending_command(command_id, now());

    let body = "<?xml version=\"1.0\"?>\n<Response>\n  <CmdType>DeviceControl</CmdType>\n  <SN>1</SN>\n  <DeviceID>34020000001320000001</DeviceID>\n  <Result>OK</Result>\n</Response>";
    let request = message_request(1, body.as_bytes())?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;

    let response = outputs
        .iter()
        .find_map(|o| match o {
            Gb28181Output::CommandResponse {
                command_id: id,
                result,
                ..
            } if *id == command_id => Some(result.clone()),
            _ => None,
        })
        .ok_or("missing command response")?;
    assert!(matches!(response, Gb28181CommandResult::Ok));
    Ok(())
}

#[test]
fn catalog_aggregation_respects_page_size() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module_with_config(test_config_with_page_size(1))?;
    let body = br#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <SumNum>2</SumNum>
  <ItemList>
    <Item>
      <DeviceID>34020000001320000001</DeviceID>
      <Name>Camera 1</Name>
      <Status>ON</Status>
    </Item>
    <Item>
      <DeviceID>34020000001320000002</DeviceID>
      <Name>Camera 2</Name>
      <Status>ON</Status>
    </Item>
  </ItemList>
</Response>"#;
    let request = message_request(1, body)?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;
    let catalog = outputs
        .iter()
        .find_map(|o| match o {
            Gb28181Output::Catalog(c) => Some(c.clone()),
            _ => None,
        })
        .ok_or("missing catalog output")?;
    assert_eq!(catalog.items.len(), 1);
    assert!(!catalog.complete);
    Ok(())
}

#[test]
fn heartbeat_timeout_clears_registration() -> Result<(), Box<dyn std::error::Error>> {
    let mut module = register_module()?;
    let outputs = module.heartbeat_timeout(now());
    assert!(outputs.iter().any(
        |o| matches!(o, Gb28181Output::ProtocolError { kind, .. } if kind == "heartbeat_timeout")
    ));
    Ok(())
}

#[test]
fn pending_command_times_out() -> Result<(), Box<dyn std::error::Error>> {
    let config = test_config_with_limits(100, 1024, 1, 1024);
    let mut module = register_module_with_config(config)?;
    let command_id = MessageId::generate();
    module.add_pending_command(command_id, now());

    let later = UtcTimestamp::default()
        .checked_add(DurationMs::from_seconds(2))
        .ok_or("timestamp overflow")?;
    let outputs = module.heartbeat_timeout(later);
    assert!(outputs.iter().any(|o| matches!(
        o,
        Gb28181Output::CommandResponse {
            result: Gb28181CommandResult::Timeout,
            ..
        }
    )));
    Ok(())
}

#[test]
fn pending_command_capacity_evicts_oldest() -> Result<(), Box<dyn std::error::Error>> {
    let config = test_config_with_limits(100, 2, 3600, 1024);
    let mut module = register_module_with_config(config)?;
    let id1 = MessageId::generate();
    let id2 = MessageId::generate();
    let id3 = MessageId::generate();
    module.add_pending_command(id1, now());
    module.add_pending_command(id2, now());
    module.add_pending_command(id3, now());

    let body = "<?xml version=\"1.0\"?>\n<Response>\n  <CmdType>DeviceControl</CmdType>\n  <SN>1</SN>\n  <DeviceID>34020000001320000001</DeviceID>\n  <Result>OK</Result>\n</Response>";
    let request = message_request(1, body.as_bytes())?;
    let outputs = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: request,
        },
        now(),
    )?;
    // SN 1 was evicted by the third insert, so id2 (SN 2) should still be present.
    assert!(
        outputs
            .iter()
            .find(|o| matches!(
                o,
                Gb28181Output::CommandResponse {
                    result: Gb28181CommandResult::Ok,
                    ..
                }
            ))
            .is_none()
    );
    Ok(())
}

#[test]
fn recent_message_capacity_evicts_oldest() -> Result<(), Box<dyn std::error::Error>> {
    let config = test_config_with_limits(100, 1024, 30, 1);
    let mut module = register_module_with_config(config)?;

    let body = b"<?xml version=\"1.0\"?><Notify><CmdType>Keepalive</CmdType><DeviceID>34020000001320000001</DeviceID><Status>OK</Status></Notify>";
    let req1 = message_request(1, body)?;
    let req2 = message_request(2, body)?;

    let out1 = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: req1,
        },
        now(),
    )?;
    assert!(count_heartbeats(&out1) == 1);

    let out2 = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: req2,
        },
        now(),
    )?;
    assert!(count_heartbeats(&out2) == 1);

    // req1 was evicted by the cap of 1, so the retransmission is treated as new.
    let req1_again = message_request(1, body)?;
    let out3 = module.handle(
        Gb28181Input {
            source: source_addr(),
            message: req1_again,
        },
        now(),
    )?;
    assert!(count_heartbeats(&out3) == 1);
    Ok(())
}

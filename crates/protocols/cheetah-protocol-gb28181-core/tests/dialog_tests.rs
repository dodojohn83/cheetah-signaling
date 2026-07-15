//! Integration tests for the Sans-I/O SIP dialog state machine.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_protocol_gb28181_core::{
    Dialog, DialogEvent, DialogId, DialogOutput, DialogRole, DialogState, SipParser,
    SipParserConfig,
};

fn parse(data: &str) -> cheetah_protocol_gb28181_core::SipMessage {
    SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default()).unwrap()
}

fn invite_request() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "INVITE sip:bob@example.com SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 2 INVITE\r\n\
        Contact: <sip:alice@192.168.1.2:5060>\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn ok_response() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "SIP/2.0 200 OK\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 2 INVITE\r\n\
        Contact: <sip:bob@192.168.1.3:5060>\r\n\
        Record-Route: <sip:proxy1.example.com;lr>\r\n\
        Record-Route: <sip:proxy2.example.com;lr>\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn bye_request() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "BYE sip:bob@192.168.1.3:5060 SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bKbye\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 3 BYE\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn bye_response() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "SIP/2.0 200 OK\r\n\
        Via: SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bKbye\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 3 BYE\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn reinvite_request() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "INVITE sip:bob@192.168.1.3:5060 SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bKreinv\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 4 INVITE\r\n\
        Contact: <sip:bob@10.0.0.1:5060>\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn ack_request(seq: u32) -> cheetah_protocol_gb28181_core::SipMessage {
    parse(&format!(
        "ACK sip:bob@192.168.1.3:5060 SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bKack{seq}\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: {seq} ACK\r\n\
        Content-Length: 0\r\n\r\n"
    ))
}

#[test]
fn uac_dialog_extracts_id_route_set_and_remote_target() {
    let dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    let id = dialog.id();

    assert_eq!(id.call_id, "call-dialog@example.com");
    assert_eq!(id.local_tag, "local-abc");
    assert_eq!(id.remote_tag, "remote-xyz");
    assert_eq!(dialog.role(), DialogRole::Uac);
    assert_eq!(dialog.state(), DialogState::Confirmed);
    assert_eq!(dialog.local_cseq(), 2);
    assert_eq!(dialog.remote_cseq(), 0);
    assert_eq!(dialog.remote_target().host(), "192.168.1.3");
    assert_eq!(dialog.route_set().len(), 2);
    assert_eq!(dialog.route_set()[0].host(), "proxy2.example.com");
    assert_eq!(dialog.route_set()[1].host(), "proxy1.example.com");
}

#[test]
fn uas_dialog_extracts_remote_tag_and_uses_local_tag() {
    let dialog = Dialog::new_uas(&invite_request(), "uas-local").unwrap();
    let id = dialog.id();

    assert_eq!(id.call_id, "call-dialog@example.com");
    assert_eq!(id.local_tag, "uas-local");
    assert_eq!(id.remote_tag, "local-abc");
    assert_eq!(dialog.role(), DialogRole::Uas);
    assert_eq!(dialog.remote_target().host(), "192.168.1.2");
    assert_eq!(dialog.route_set().len(), 0);
}

#[test]
fn uac_bye_terminates_dialog() {
    let mut dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    let outputs = dialog.process(DialogEvent::Request(bye_request()));

    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, DialogOutput::Deliver(_)))
    );
    assert!(outputs.iter().any(|o| matches!(o, DialogOutput::Complete)));
    assert!(dialog.is_terminated());
    assert_eq!(dialog.state(), DialogState::Terminated);
}

#[test]
fn bye_response_terminates_dialog() {
    let mut dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    let outputs = dialog.process(DialogEvent::Response(bye_response()));

    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, DialogOutput::Deliver(_)))
    );
    assert!(outputs.iter().any(|o| matches!(o, DialogOutput::Complete)));
    assert!(dialog.is_terminated());
}

#[test]
fn reinvite_updates_remote_target_and_cseq() {
    let mut dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    let outputs = dialog.process(DialogEvent::Request(reinvite_request()));

    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, DialogOutput::Deliver(_)))
    );
    assert_eq!(dialog.remote_cseq(), 4);
    assert_eq!(dialog.remote_target().host(), "10.0.0.1");
}

#[test]
fn ack_for_2xx_invite_is_delivered_without_cseq_check() {
    let mut dialog = Dialog::new_uas(&invite_request(), "uas-local").unwrap();
    // remote_cseq is the INVITE's CSeq (2). The ACK for the 2xx reuses CSeq 2.
    let outputs = dialog.process(DialogEvent::Request(ack_request(2)));
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, DialogOutput::Deliver(_)))
    );
    assert_eq!(dialog.remote_cseq(), 2);
    assert!(!dialog.is_terminated());
}

#[test]
fn ack_for_reinvite_is_delivered_without_cseq_check() {
    let mut dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    let _ = dialog.process(DialogEvent::Request(reinvite_request()));
    assert_eq!(dialog.remote_cseq(), 4);

    let outputs = dialog.process(DialogEvent::Request(ack_request(4)));
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, DialogOutput::Deliver(_)))
    );
    assert_eq!(dialog.remote_cseq(), 4);
}

#[test]
fn out_of_order_request_is_absorbed() {
    let mut dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    // remote_cseq starts at 0; accept a valid request with CSeq 3.
    let first = parse(
        "MESSAGE sip:bob@192.168.1.3:5060 SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bKmsg\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 3 MESSAGE\r\n\
        Content-Length: 0\r\n\r\n",
    );
    assert!(
        dialog
            .process(DialogEvent::Request(first))
            .iter()
            .any(|o| matches!(o, DialogOutput::Deliver(_)))
    );
    assert_eq!(dialog.remote_cseq(), 3);

    // A later stale request with CSeq 2 must be absorbed.
    let stale = parse(
        "MESSAGE sip:bob@192.168.1.3:5060 SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bKstale\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 2 MESSAGE\r\n\
        Content-Length: 0\r\n\r\n",
    );
    let outputs = dialog.process(DialogEvent::Request(stale));
    assert!(outputs.is_empty());
    assert_eq!(dialog.remote_cseq(), 3);
}

#[test]
fn out_of_order_bye_is_absorbed() {
    let mut dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    // Accept a request with CSeq 3 first.
    let first = parse(
        "MESSAGE sip:bob@192.168.1.3:5060 SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bKmsg\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 3 MESSAGE\r\n\
        Content-Length: 0\r\n\r\n",
    );
    assert!(
        dialog
            .process(DialogEvent::Request(first))
            .iter()
            .any(|o| matches!(o, DialogOutput::Deliver(_)))
    );
    assert_eq!(dialog.remote_cseq(), 3);

    // A stale BYE with a lower CSeq must not terminate the dialog.
    let stale_bye = parse(
        "BYE sip:bob@192.168.1.3:5060 SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bKbye2\r\n\
        From: <sip:alice@example.com>;tag=local-abc\r\n\
        To: <sip:bob@example.com>;tag=remote-xyz\r\n\
        Call-ID: call-dialog@example.com\r\n\
        CSeq: 2 BYE\r\n\
        Content-Length: 0\r\n\r\n",
    );
    assert!(dialog.process(DialogEvent::Request(stale_bye)).is_empty());
    assert!(!dialog.is_terminated());
    assert_eq!(dialog.remote_cseq(), 3);
}

#[test]
fn timer_terminates_dialog() {
    let mut dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    let outputs = dialog.process(DialogEvent::Timer);
    assert!(outputs.iter().any(|o| matches!(o, DialogOutput::Complete)));
    assert!(dialog.is_terminated());
}

#[test]
fn next_local_cseq_increments() {
    let mut dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    assert_eq!(dialog.next_local_cseq(), 3);
    assert_eq!(dialog.local_cseq(), 3);
}

#[test]
fn dialog_id_matches_call_id_and_tags() {
    let dialog = Dialog::new_uac(&invite_request(), &ok_response()).unwrap();
    let id = dialog.id();
    assert_eq!(
        *id,
        DialogId {
            call_id: "call-dialog@example.com".to_string(),
            local_tag: "local-abc".to_string(),
            remote_tag: "remote-xyz".to_string(),
        }
    );
}

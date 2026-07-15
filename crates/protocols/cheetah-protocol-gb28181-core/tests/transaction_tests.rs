//! Integration tests for the Sans-I/O SIP transaction state machine.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_protocol_gb28181_core::{
    SipParser, SipParserConfig, TimerKind, Transaction, TransactionConfig, TransactionEvent,
    TransactionOutput,
};
use std::time::Duration;

fn parse(data: &str) -> cheetah_protocol_gb28181_core::SipMessage {
    SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default()).unwrap()
}

fn invite_request() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "INVITE sip:bob@example.com SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
        From: <sip:alice@example.com>;tag=abc\r\n\
        To: <sip:bob@example.com>\r\n\
        Call-ID: call-2@example.com\r\n\
        CSeq: 2 INVITE\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn ack_request() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "ACK sip:bob@example.com SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
        From: <sip:alice@example.com>;tag=abc\r\n\
        To: <sip:bob@example.com>;tag=remote\r\n\
        Call-ID: call-2@example.com\r\n\
        CSeq: 2 ACK\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn cancel_request() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "CANCEL sip:bob@example.com SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
        From: <sip:alice@example.com>;tag=abc\r\n\
        To: <sip:bob@example.com>\r\n\
        Call-ID: call-2@example.com\r\n\
        CSeq: 2 CANCEL\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn invite_response_100() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "SIP/2.0 100 Trying\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
        From: <sip:alice@example.com>;tag=abc\r\n\
        To: <sip:bob@example.com>\r\n\
        Call-ID: call-2@example.com\r\n\
        CSeq: 2 INVITE\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn invite_response_200() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "SIP/2.0 200 OK\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
        From: <sip:alice@example.com>;tag=abc\r\n\
        To: <sip:bob@example.com>;tag=remote\r\n\
        Call-ID: call-2@example.com\r\n\
        CSeq: 2 INVITE\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn invite_response_487() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "SIP/2.0 487 Request Terminated\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
        From: <sip:alice@example.com>;tag=abc\r\n\
        To: <sip:bob@example.com>;tag=remote\r\n\
        Call-ID: call-2@example.com\r\n\
        CSeq: 2 INVITE\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn register_request() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "REGISTER sip:registrar.example.com SIP/2.0\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKregister\r\n\
        From: <sip:alice@example.com>;tag=abc\r\n\
        To: <sip:alice@example.com>\r\n\
        Call-ID: call-1@example.com\r\n\
        CSeq: 1 REGISTER\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

fn register_response_200() -> cheetah_protocol_gb28181_core::SipMessage {
    parse(
        "SIP/2.0 200 OK\r\n\
        Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKregister\r\n\
        From: <sip:alice@example.com>;tag=abc\r\n\
        To: <sip:alice@example.com>;tag=reg\r\n\
        Call-ID: call-1@example.com\r\n\
        CSeq: 1 REGISTER\r\n\
        Content-Length: 0\r\n\r\n",
    )
}

#[test]
fn client_invite_bootstrap_sends_request_and_arms_timers() {
    let txn = Transaction::client_invite(invite_request(), TransactionConfig::default())
        .expect("valid INVITE request");
    let outputs = match txn {
        Transaction::Client(mut t) => t.bootstrap(Duration::ZERO),
        _ => panic!("expected client"),
    };

    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::SendMessage(_)))
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::A,
            ..
        }
    )));
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::B,
            ..
        }
    )));
}

#[test]
fn client_invite_success_terminates() {
    let txn = Transaction::client_invite(invite_request(), TransactionConfig::default())
        .expect("valid INVITE request");
    let mut client = match txn {
        Transaction::Client(t) => t,
        _ => panic!("expected client"),
    };
    let _ = client.bootstrap(Duration::ZERO);

    let outputs = client.process(
        TransactionEvent::Response(invite_response_200()),
        Duration::from_secs(1),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Deliver(_)))
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Complete))
    );
    assert!(client.is_terminated());
}

#[test]
fn client_invite_non_2xx_sends_ack_and_waits_for_timer_d() {
    let txn = Transaction::client_invite(invite_request(), TransactionConfig::default())
        .expect("valid INVITE request");
    let mut client = match txn {
        Transaction::Client(t) => t,
        _ => panic!("expected client"),
    };
    let _ = client.bootstrap(Duration::ZERO);

    let outputs = client.process(
        TransactionEvent::Response(invite_response_487()),
        Duration::from_secs(1),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Deliver(_)))
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::SendMessage(_)))
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::D,
            ..
        }
    )));
    assert!(!client.is_terminated());

    // Retransmitted final response should retransmit ACK.
    let outputs = client.process(
        TransactionEvent::Response(invite_response_487()),
        Duration::from_secs(2),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::SendMessage(_)))
    );

    let outputs = client.process(
        TransactionEvent::Timer(TimerKind::D),
        Duration::from_secs(100),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Complete))
    );
    assert!(client.is_terminated());
}

#[test]
fn client_invite_timer_b_timeout() {
    let txn = Transaction::client_invite(invite_request(), TransactionConfig::default())
        .expect("valid INVITE request");
    let mut client = match txn {
        Transaction::Client(t) => t,
        _ => panic!("expected client"),
    };
    let _ = client.bootstrap(Duration::ZERO);

    let outputs = client.process(
        TransactionEvent::Timer(TimerKind::B),
        Duration::from_secs(40),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Failure(_)))
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Complete))
    );
    assert!(client.is_terminated());
}

#[test]
fn client_non_invite_retransmits_and_completes_on_final() {
    let txn = Transaction::new_client(register_request(), TransactionConfig::default())
        .expect("valid request");
    let mut client = match txn {
        Transaction::Client(t) => t,
        _ => panic!("expected client"),
    };
    let _ = client.bootstrap(Duration::ZERO);

    let outputs = client.process(
        TransactionEvent::Timer(TimerKind::E),
        Duration::from_millis(500),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::SendMessage(_)))
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::E,
            ..
        }
    )));

    let outputs = client.process(
        TransactionEvent::Response(register_response_200()),
        Duration::from_secs(2),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Deliver(_)))
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::K,
            ..
        }
    )));

    let outputs = client.process(
        TransactionEvent::Timer(TimerKind::K),
        Duration::from_secs(10),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Complete))
    );
    assert!(client.is_terminated());
}

#[test]
fn server_non_invite_delivers_request_and_completes_on_final() {
    let txn = Transaction::new_server(register_request(), TransactionConfig::default())
        .expect("valid request");
    let mut server = match txn {
        Transaction::Server(t) => t,
        _ => panic!("expected server"),
    };
    let outputs = server.bootstrap();
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Deliver(_)))
    );

    let outputs = server.process(
        TransactionEvent::Response(register_response_200()),
        Duration::from_secs(1),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::SendMessage(_)))
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::J,
            ..
        }
    )));

    // Request retransmission in Completed retransmits final response.
    let outputs = server.process(
        TransactionEvent::Request(register_request()),
        Duration::from_secs(2),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::SendMessage(_)))
    );

    let outputs = server.process(
        TransactionEvent::Timer(TimerKind::J),
        Duration::from_secs(70),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Complete))
    );
}

#[test]
fn server_invite_final_and_ack_lifecycle() {
    let txn = Transaction::server_invite(invite_request(), TransactionConfig::default())
        .expect("valid INVITE request");
    let mut server = match txn {
        Transaction::Server(t) => t,
        _ => panic!("expected server"),
    };
    let _ = server.bootstrap();

    // TU sends 100 Trying.
    let outputs = server.process(
        TransactionEvent::Response(invite_response_100()),
        Duration::from_millis(50),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::SendMessage(_)))
    );

    // TU sends 487 final.
    let outputs = server.process(
        TransactionEvent::Response(invite_response_487()),
        Duration::from_millis(100),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::SendMessage(_)))
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::G,
            ..
        }
    )));
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::H,
            ..
        }
    )));

    // Client ACK.
    let outputs = server.process(
        TransactionEvent::Request(ack_request()),
        Duration::from_millis(150),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Deliver(_)))
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        TransactionOutput::SetTimer {
            kind: TimerKind::I,
            ..
        }
    )));

    let outputs = server.process(
        TransactionEvent::Timer(TimerKind::I),
        Duration::from_secs(10),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Complete))
    );
}

#[test]
fn server_invite_cancel_is_delivered_to_tu() {
    let txn = Transaction::server_invite(invite_request(), TransactionConfig::default())
        .expect("valid INVITE request");
    let mut server = match txn {
        Transaction::Server(t) => t,
        _ => panic!("expected server"),
    };
    let _ = server.bootstrap();

    let outputs = server.process(
        TransactionEvent::Request(cancel_request()),
        Duration::from_millis(50),
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Deliver(_)))
    );
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, TransactionOutput::Failure(_)))
    );
}

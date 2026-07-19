//! Integration tests for the Sans-I/O SIP parser and encoder.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_gb28181_core::{
    HeaderName, Method, ResponseClass, SipErrorKind, SipMessage, SipParser, SipParserConfig,
    encode_message,
};

const REGISTER: &str = "REGISTER sip:registrar.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.example.com;branch=z9hG4bK776asdhds\r\n\
Max-Forwards: 70\r\n\
From: <sip:alice@example.com>;tag=1928301774\r\n\
To: <sip:alice@example.com>\r\n\
Call-ID: a84b4c76e66710@pc33.example.com\r\n\
CSeq: 314159 REGISTER\r\n\
Contact: <sip:alice@pc33.example.com>\r\n\
Content-Length: 0\r\n\r\n";

#[test]
fn parse_register_request() {
    let msg = SipParser::parse_datagram(REGISTER.as_bytes(), SipParserConfig::default())
        .expect("REGISTER should parse");
    let SipMessage::Request {
        line,
        headers,
        body,
    } = msg
    else {
        panic!("expected a request");
    };
    assert!(matches!(line.method, Method::Register));
    assert_eq!(line.uri.host(), "registrar.example.com");
    assert_eq!(
        headers.get(&HeaderName::CallId).unwrap().as_str(),
        "a84b4c76e66710@pc33.example.com"
    );
    assert_eq!(body.len(), 0);
}

#[test]
fn parse_response_and_classify() {
    let data = "SIP/2.0 200 OK\r\n\
Call-ID: a84b4c76e66710@pc33.example.com\r\n\
CSeq: 314159 REGISTER\r\n\
Content-Length: 0\r\n\r\n";
    let msg = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect("response should parse");
    let SipMessage::Response { line, .. } = msg else {
        panic!("expected a response");
    };
    assert_eq!(line.code, 200);
    assert_eq!(line.class(), ResponseClass::Success);
}

#[test]
fn round_trip_register() {
    let original = SipParser::parse_datagram(REGISTER.as_bytes(), SipParserConfig::default())
        .expect("REGISTER should parse");
    let encoded = encode_message(&original);
    let round_tripped =
        SipParser::parse_datagram(&encoded, SipParserConfig::default()).expect("round trip parse");

    // Header count and start-line fields should be preserved.
    match (&original, &round_tripped) {
        (SipMessage::Request { line: l1, .. }, SipMessage::Request { line: l2, .. }) => {
            assert_eq!(l1.method.to_string(), l2.method.to_string());
            assert_eq!(l1.uri.encode(), l2.uri.encode());
        }
        _ => panic!("both should be requests"),
    }
    assert_eq!(
        original
            .headers()
            .get(&HeaderName::CallId)
            .unwrap()
            .as_str(),
        round_tripped
            .headers()
            .get(&HeaderName::CallId)
            .unwrap()
            .as_str(),
    );
}

#[test]
fn tcp_stream_handles_partial_and_multiple_messages() {
    let mut parser = SipParser::new(SipParserConfig::default());

    let data = REGISTER.as_bytes();
    // Feed first half
    parser.feed(&data[..data.len() / 2]).unwrap();
    assert!(parser.pop_message().is_none());

    // Feed remaining plus a second full message
    let second = "SIP/2.0 100 Trying\r\nContent-Length: 0\r\n\r\n";
    parser.feed(&data[data.len() / 2..]).unwrap();
    parser.feed(second.as_bytes()).unwrap();

    let first = parser.pop_message().unwrap().expect("first message");
    assert!(matches!(first, SipMessage::Request { .. }));

    let second = parser.pop_message().unwrap().expect("second message");
    let SipMessage::Response { line, .. } = second else {
        panic!("expected response");
    };
    assert_eq!(line.code, 100);
}

#[test]
fn content_length_must_match_body_bytes() {
    let data = "SIP/2.0 200 OK\r\nContent-Length: 5\r\n\r\n1234";
    let err = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect_err("should reject short body");
    assert!(matches!(
        err.kind,
        cheetah_gb28181_core::SipErrorKind::ContentLengthMismatch
    ));
}

#[test]
fn large_body_with_short_start_line_parses() {
    let body = "x".repeat(8192);
    let data = format!(
        "SIP/2.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let msg = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect("should parse large body");
    assert_eq!(msg.body().len(), 8192);
}

#[test]
fn oversized_start_line_is_rejected() {
    let config = SipParserConfig {
        max_start_line_bytes: 32,
        ..SipParserConfig::default()
    };
    let data = format!(
        "SIP/2.0 200 {}\r\nContent-Length: 0\r\n\r\n",
        "x".repeat(64)
    );
    let err = SipParser::parse_datagram(data.as_bytes(), config)
        .expect_err("should reject long start line");
    assert!(matches!(
        err.kind,
        cheetah_gb28181_core::SipErrorKind::StartLineTooLong
    ));
}

#[test]
fn malformed_header_missing_colon_is_rejected() {
    let data = "SIP/2.0 200 OK\r\nNoColonValue\r\nContent-Length: 0\r\n\r\n";
    let err = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect_err("should reject malformed header");
    assert!(matches!(
        err.kind,
        cheetah_gb28181_core::SipErrorKind::InvalidHeader
    ));
}

#[test]
fn parser_buffer_size_is_limited() {
    let config = SipParserConfig {
        max_buffer_bytes: 32,
        ..SipParserConfig::default()
    };
    let mut parser = SipParser::new(config);
    let result = parser.feed(b"this is a very long stream of bytes without any crlf");
    assert!(matches!(
        result.unwrap_err().kind,
        cheetah_gb28181_core::SipErrorKind::BufferTooLarge
    ));
}

#[test]
fn compact_header_forms_are_recognized() {
    let data = "SIP/2.0 200 OK\r\n\
        v: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK123\r\n\
        f: <sip:alice@example.com>\r\n\
        t: <sip:bob@example.com>\r\n\
        s: Presence\r\n\
        i: call-123@example.com\r\n\
        l: 0\r\n\r\n";
    let msg = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect("should parse compact headers");
    assert!(msg.headers().get(&HeaderName::Via).is_some());
    assert!(msg.headers().get(&HeaderName::From).is_some());
    assert!(msg.headers().get(&HeaderName::To).is_some());
    assert!(msg.headers().get(&HeaderName::Subject).is_some());
    assert!(msg.headers().get(&HeaderName::CallId).is_some());
    assert!(msg.headers().get(&HeaderName::ContentLength).is_some());
    assert_eq!(msg.body().len(), 0);
}

#[test]
fn header_values_are_trimmed() {
    let data = "SIP/2.0 200 OK\r\nCall-ID: call-abc \r\nContent-Length: 0\r\n\r\n";
    let msg = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect("should parse");
    let name = HeaderName::CallId;
    let value = msg.headers().get(&name).expect("Call-ID present");
    assert_eq!(value.as_str(), "call-abc");
}

#[test]
fn unknown_header_lookup_is_case_insensitive() {
    let data = "SIP/2.0 200 OK\r\nX-Custom: value\r\nContent-Length: 0\r\n\r\n";
    let msg = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect("should parse");
    let lower = HeaderName::Other("x-custom".to_string());
    let value = msg
        .headers()
        .get(&lower)
        .expect("lookup by lowercase should find");
    assert_eq!(value.as_str(), "value");
}

#[test]
fn unknown_header_is_preserved() {
    let data = "SIP/2.0 200 OK\r\nX-Custom: value\r\nContent-Length: 0\r\n\r\n";
    let msg = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect("should parse");
    let name = HeaderName::Other("X-Custom".to_string());
    assert_eq!(msg.headers().get(&name).unwrap().as_str(), "value");
}

#[test]
fn invalid_content_length_is_rejected() {
    let data = "SIP/2.0 200 OK\r\nCall-ID: call-abc\r\nContent-Length: not-a-number\r\n\r\n";
    let err = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect_err("non-numeric Content-Length should fail");
    assert_eq!(err.kind, SipErrorKind::InvalidHeader);
}

#[test]
fn empty_content_length_is_rejected() {
    let data = "SIP/2.0 200 OK\r\nCall-ID: call-abc\r\nContent-Length:\r\n\r\n";
    let err = SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
        .expect_err("empty Content-Length should fail");
    assert_eq!(err.kind, SipErrorKind::InvalidHeader);
}

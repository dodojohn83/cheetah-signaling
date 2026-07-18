//! Property-based / short-fuzz tests for GB28181 SIP and SDP parsers.
//!
//! These tests verify the parser invariants required by TST-004: no panic,
//! bounded allocation, round-trip semantics, and rejection of ambiguous lengths.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_gb28181_core::{
    SdpParserConfig, SipErrorKind, SipMessage, SipParser, SipParserConfig, encode_message,
    parse_sdp,
};
use proptest::prelude::*;

const VALID_REGISTER: &str = "REGISTER sip:registrar.example.com SIP/2.0\r\
                               \nVia: SIP/2.0/UDP pc33.example.com;branch=z9hG4bK776asdhds\r\
                               \nMax-Forwards: 70\r\
                               \nFrom: <sip:alice@example.com>;tag=1928301774\r\
                               \nTo: <sip:alice@example.com>\r\
                               \nCall-ID: a84b4c76e66710@pc33.example.com\r\
                               \nCSeq: 314159 REGISTER\r\
                               \nContact: <sip:alice@pc33.example.com>\r\
                               \nContent-Length: 0\r\n\r\n";

fn valid_register_message() -> SipMessage {
    SipParser::parse_datagram(VALID_REGISTER.as_bytes(), SipParserConfig::default()).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// SIP parser must not panic on arbitrary byte input.
    #[test]
    fn sip_parse_never_panics(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = SipParser::parse_datagram(&data, SipParserConfig::default());
    }

    /// SDP parser must not panic on arbitrary byte input.
    #[test]
    fn sdp_parse_never_panics(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = parse_sdp(&data, &SdpParserConfig::default());
    }

    /// A valid SIP message round-trips through encode and re-parse.
    #[test]
    fn sip_round_trip_preserves_message(_seed in 0usize..1) {
        let original = valid_register_message();
        let encoded = encode_message(&original);
        let round = SipParser::parse_datagram(&encoded, SipParserConfig::default()).unwrap();

        // Start-line method and URI are preserved.
        match (&original, &round) {
            (SipMessage::Request { line: l1, .. }, SipMessage::Request { line: l2, .. }) => {
                assert_eq!(l1.method.to_string(), l2.method.to_string());
                assert_eq!(l1.uri.encode(), l2.uri.encode());
            }
            _ => panic!("expected request round-trip"),
        }
    }

    /// Mismatched Content-Length is rejected rather than silently truncated or padded.
    #[test]
    fn sip_rejects_ambiguous_content_length(
        body in prop::collection::vec(any::<u8>(), 0..256),
        declared in 0usize..512usize,
    ) {
        let header = format!(
            "SIP/2.0 200 OK\r\nContent-Length: {}\r\n\r\n",
            declared
        );
        let mut data = header.into_bytes();
        data.extend_from_slice(&body);

        let result = SipParser::parse_datagram(&data, SipParserConfig::default());
        if declared == body.len() {
            // Exact length should parse; body bytes are preserved.
            let msg = result.expect("should parse when length matches");
            assert_eq!(msg.body().len(), body.len());
        } else {
            // Mismatch must be reported: too short produces ContentLengthMismatch,
            // too long leaves trailing bytes and produces InvalidFraming.
            let err = result.expect_err("should reject mismatched content length");
            assert!(matches!(err.kind, SipErrorKind::ContentLengthMismatch | SipErrorKind::InvalidFraming));
        }
    }

    /// SDP parser rejects bodies that exceed configured size limits.
    #[test]
    fn sdp_rejects_oversized_bodies(body in prop::collection::vec(any::<u8>(), 1025..2048)) {
        let config = SdpParserConfig {
            max_size: 1024,
            ..SdpParserConfig::default()
        };
        let result = parse_sdp(&body, &config);
        assert!(result.is_err());
    }
}

//! Deterministic in-memory transport ingestion.
//!
//! For UDP each delivery is an independent datagram parsed with
//! [`SipParser::parse_datagram`].  For TCP each ordered endpoint pair owns a
//! streaming [`SipParser`]; fragments (including half-packets injected by the
//! fault engine) are fed incrementally and complete messages are popped as they
//! reassemble, exercising the real parser contract for coalesced/split framing.

use crate::scenario::Transport;
use crate::wire::Endpoint;
use cheetah_gb28181_core::{SipMessage, SipParser, SipParserConfig};
use std::collections::HashMap;

/// Result of ingesting one delivery.
#[derive(Debug, Default)]
pub struct Ingested {
    /// Fully parsed messages ready for dispatch.
    pub messages: Vec<SipMessage>,
    /// Number of parse errors observed (malformed/truncated input).
    pub parse_errors: u64,
}

/// Reassembles deliveries into parsed SIP messages per transport semantics.
#[derive(Debug)]
pub struct Assembler {
    transport: Transport,
    config: SipParserConfig,
    streams: HashMap<(Endpoint, Endpoint), SipParser>,
}

impl Assembler {
    /// Creates an assembler for the given transport.
    pub fn new(transport: Transport) -> Self {
        Self {
            transport,
            config: SipParserConfig::default(),
            streams: HashMap::new(),
        }
    }

    /// Ingests one delivery of `bytes` on the `from -> to` edge.
    ///
    /// When `split` is set on a TCP stream, the bytes are fed to the parser in
    /// two chunks to exercise incremental half-packet reassembly.
    pub fn ingest(&mut self, from: Endpoint, to: Endpoint, bytes: &[u8], split: bool) -> Ingested {
        match self.transport {
            Transport::Udp => self.ingest_datagram(bytes),
            Transport::Tcp => self.ingest_stream(from, to, bytes, split),
        }
    }

    fn ingest_datagram(&self, bytes: &[u8]) -> Ingested {
        match SipParser::parse_datagram(bytes, self.config) {
            Ok(msg) => Ingested {
                messages: vec![msg],
                parse_errors: 0,
            },
            Err(_) => Ingested {
                messages: Vec::new(),
                parse_errors: 1,
            },
        }
    }

    fn ingest_stream(
        &mut self,
        from: Endpoint,
        to: Endpoint,
        bytes: &[u8],
        split: bool,
    ) -> Ingested {
        let parser = self
            .streams
            .entry((from, to))
            .or_insert_with(|| SipParser::new(self.config));
        let mut result = Ingested::default();
        let chunks: Vec<&[u8]> = if split && bytes.len() >= 2 {
            let mid = bytes.len() / 2;
            vec![&bytes[..mid], &bytes[mid..]]
        } else {
            vec![bytes]
        };
        for chunk in chunks {
            if parser.feed(chunk).is_err() {
                result.parse_errors += 1;
                return result;
            }
        }
        loop {
            match parser.pop_message() {
                Some(Ok(msg)) => result.messages.push(msg),
                Some(Err(_)) => {
                    result.parse_errors += 1;
                    break;
                }
                None => break,
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use cheetah_gb28181_core::{
        Method, RequestLine, SipHeaders, SipMessage, SipUri, encode_message,
    };

    fn sample() -> Vec<u8> {
        let uri = SipUri::parse("sip:platform").unwrap();
        let mut headers = SipHeaders::new();
        headers.append(
            cheetah_gb28181_core::HeaderName::CallId,
            cheetah_gb28181_core::HeaderValue::new("abc"),
        );
        let msg = SipMessage::Request {
            line: RequestLine::new(Method::Register, uri),
            headers,
            body: Vec::new(),
        };
        encode_message(&msg)
    }

    #[test]
    fn udp_parses_whole_datagram() {
        let mut asm = Assembler::new(Transport::Udp);
        let ingested = asm.ingest(Endpoint::Device(0), Endpoint::Platform, &sample(), false);
        assert_eq!(ingested.messages.len(), 1);
        assert_eq!(ingested.parse_errors, 0);
    }

    #[test]
    fn udp_reports_parse_error_on_corruption() {
        let mut asm = Assembler::new(Transport::Udp);
        let mut bytes = sample();
        bytes[0] = 0;
        let ingested = asm.ingest(Endpoint::Device(0), Endpoint::Platform, &bytes, false);
        assert_eq!(ingested.messages.len(), 0);
        assert_eq!(ingested.parse_errors, 1);
    }

    #[test]
    fn tcp_reassembles_half_packets() {
        let mut asm = Assembler::new(Transport::Tcp);
        let bytes = sample();
        let mid = bytes.len() / 2;
        let first = asm.ingest(
            Endpoint::Device(0),
            Endpoint::Platform,
            &bytes[..mid],
            false,
        );
        assert_eq!(first.messages.len(), 0);
        let second = asm.ingest(
            Endpoint::Device(0),
            Endpoint::Platform,
            &bytes[mid..],
            false,
        );
        assert_eq!(second.messages.len(), 1);
    }
}

//! Sans-I/O SIP parser for datagram and stream transports.

use super::error::{SipError, SipErrorKind};
use super::headers::{HeaderName, HeaderValue, SipHeaders};
use super::message::{Body, Method, RequestLine, SipMessage, StatusLine};
use super::uri::SipUri;
use crate::{CompatibilityCapability, CompatibilityProfile};

/// Limits and behavior of the SIP parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SipParserConfig {
    /// Maximum start line length in bytes.
    pub max_start_line_bytes: usize,
    /// Maximum number of headers.
    pub max_headers: usize,
    /// Maximum total header block size in bytes.
    pub max_header_block_bytes: usize,
    /// Maximum individual header line length in bytes.
    pub max_header_line_bytes: usize,
    /// Maximum body size in bytes.
    pub max_body_bytes: usize,
    /// Maximum total parser buffer size in bytes.
    pub max_buffer_bytes: usize,
    /// Whether the input is a complete UDP datagram. In datagram mode a missing
    /// `Content-Length` means the body is the remainder of the datagram; in stream
    /// mode a missing `Content-Length` is an error because the parser cannot
    /// determine message boundaries.
    pub datagram_mode: bool,
}

impl Default for SipParserConfig {
    fn default() -> Self {
        Self {
            max_start_line_bytes: 4096,
            max_headers: 256,
            max_header_block_bytes: 65536,
            max_header_line_bytes: 4096,
            max_body_bytes: 2_097_152,
            max_buffer_bytes: 8_388_608,
            datagram_mode: false,
        }
    }
}

/// Parser state for incremental TCP framing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum ParserState {
    #[default]
    StartLine,
    Headers {
        start_line: String,
    },
    Body {
        start_line: String,
        headers: SipHeaders,
        content_length: usize,
        header_bytes: usize,
    },
}

/// Sans-I/O SIP parser supporting UDP datagrams and TCP streams.
#[derive(Clone, Debug)]
pub struct SipParser {
    config: SipParserConfig,
    profile: Option<CompatibilityProfile>,
    state: ParserState,
    buffer: Vec<u8>,
}

impl SipParser {
    /// Creates a parser with the given limits.
    pub fn new(config: SipParserConfig) -> Self {
        Self::new_with_profile(config, None)
    }

    /// Creates a parser with the given limits and an optional compatibility
    /// profile that gates non-ambiguous header normalization.
    pub fn new_with_profile(
        config: SipParserConfig,
        profile: Option<CompatibilityProfile>,
    ) -> Self {
        Self {
            config,
            profile,
            state: ParserState::default(),
            buffer: Vec::new(),
        }
    }

    /// Parses a complete UDP datagram, returning the message and any trailing bytes.
    ///
    /// # Errors
    ///
    /// Returns `SipError` for malformed or oversized messages.
    pub fn parse_datagram(data: &[u8], config: SipParserConfig) -> Result<SipMessage, SipError> {
        Self::parse_datagram_with_profile(data, config, None)
    }

    /// Parses a complete UDP datagram with a compatibility profile.
    ///
    /// # Errors
    ///
    /// Returns `SipError` for malformed or oversized messages.
    pub fn parse_datagram_with_profile(
        data: &[u8],
        mut config: SipParserConfig,
        profile: Option<&CompatibilityProfile>,
    ) -> Result<SipMessage, SipError> {
        config.datagram_mode = true;
        let mut parser = Self::new_with_profile(config, profile.cloned());
        parser.feed(data)?;
        match parser.pop_message() {
            Some(Ok(message)) => {
                if !parser.buffer.is_empty() {
                    return Err(SipError::new(
                        SipErrorKind::InvalidFraming,
                        None,
                        format!(
                            "trailing bytes after Content-Length: {} bytes remain",
                            parser.buffer.len()
                        ),
                    ));
                }
                Ok(message)
            }
            Some(Err(e)) => Err(e),
            None => {
                if let ParserState::Body { content_length, .. } = &parser.state
                    && parser.buffer.len() < *content_length
                {
                    return Err(SipError::new(
                        SipErrorKind::ContentLengthMismatch,
                        None,
                        format!(
                            "Content-Length {content_length} exceeds datagram body {}",
                            parser.buffer.len()
                        ),
                    ));
                }
                Err(SipError::new(
                    SipErrorKind::InvalidFraming,
                    None,
                    "incomplete datagram",
                ))
            }
        }
    }

    /// Appends incoming bytes to the stream buffer.
    ///
    /// # Errors
    ///
    /// Returns `SipErrorKind::BufferTooLarge` if the buffer would exceed the
    /// configured `max_buffer_bytes`.
    pub fn feed(&mut self, data: &[u8]) -> Result<(), SipError> {
        if self.buffer.len() + data.len() > self.config.max_buffer_bytes {
            return Err(SipError::new(
                SipErrorKind::BufferTooLarge,
                None,
                "parser buffer exceeds limit",
            ));
        }
        self.buffer.extend_from_slice(data);
        Ok(())
    }

    /// Attempts to extract a complete message from the stream buffer.
    ///
    /// Returns `None` when more data is needed.
    pub fn pop_message(&mut self) -> Option<Result<SipMessage, SipError>> {
        let state = self.state.clone();
        match state {
            ParserState::StartLine => {
                if let Some(result) = self.parse_start_line() {
                    match result {
                        Ok(start_line) => {
                            self.state = ParserState::Headers { start_line };
                            return self.pop_message();
                        }
                        Err(e) => {
                            self.buffer.clear();
                            self.state = ParserState::StartLine;
                            return Some(Err(e));
                        }
                    }
                }
                None
            }
            ParserState::Headers { start_line } => {
                if let Some(result) = self.parse_headers() {
                    match result {
                        Ok((headers, content_length, header_bytes)) => {
                            self.state = ParserState::Body {
                                start_line,
                                headers,
                                content_length,
                                header_bytes,
                            };
                            return self.pop_message();
                        }
                        Err(e) => {
                            self.buffer.clear();
                            self.state = ParserState::StartLine;
                            return Some(Err(e));
                        }
                    }
                }
                None
            }
            ParserState::Body {
                start_line,
                headers,
                content_length,
                ..
            } => {
                if self.buffer.len() < content_length {
                    return None;
                }
                let body = self.buffer[..content_length].to_vec();
                let result = build_message(
                    &start_line,
                    &headers,
                    body,
                    content_length,
                    self.profile.as_ref(),
                );
                self.buffer.drain(..content_length);
                self.state = ParserState::StartLine;
                Some(result)
            }
        }
    }

    fn parse_start_line(&mut self) -> Option<Result<String, SipError>> {
        'outer: loop {
            for i in 0..self.buffer.len().saturating_sub(1) {
                if i > self.config.max_start_line_bytes {
                    return Some(Err(SipError::new(
                        SipErrorKind::StartLineTooLong,
                        Some(i),
                        "start line exceeds limit",
                    )));
                }
                if self.buffer[i] == b'\r' && self.buffer[i + 1] == b'\n' {
                    let line = match std::str::from_utf8(&self.buffer[..i]) {
                        Ok(s) => s.to_string(),
                        Err(_) => {
                            return Some(Err(SipError::new(
                                SipErrorKind::InvalidStartLine,
                                Some(i),
                                "start line is not valid UTF-8",
                            )));
                        }
                    };
                    self.buffer.drain(..i + 2);
                    if line.is_empty() {
                        // Skip empty leading line (common for CRLF keep-alive).
                        continue 'outer;
                    }
                    return Some(Ok(line));
                }
            }
            if self.buffer.len() > self.config.max_start_line_bytes {
                return Some(Err(SipError::new(
                    SipErrorKind::StartLineTooLong,
                    None,
                    "start line exceeds limit",
                )));
            }
            return None;
        }
    }

    fn parse_headers(&mut self) -> Option<Result<(SipHeaders, usize, usize), SipError>> {
        let mut headers = SipHeaders::new();
        let mut consumed = 0usize;
        let mut header_count = 0usize;
        let mut header_block_size = 0usize;

        loop {
            if header_block_size > self.config.max_header_block_bytes {
                return Some(Err(SipError::new(
                    SipErrorKind::HeadersTooLarge,
                    Some(consumed),
                    "header block exceeds limit",
                )));
            }
            if header_count > self.config.max_headers {
                return Some(Err(SipError::new(
                    SipErrorKind::TooManyHeaders,
                    Some(consumed),
                    "too many headers",
                )));
            }
            if consumed + 1 >= self.buffer.len() {
                return None;
            }
            if self.buffer[consumed] == b'\r' && self.buffer.get(consumed + 1) == Some(&b'\n') {
                // With HeaderNormalization, an empty line inside the header block
                // may be followed by more headers; only treat it as the terminator
                // when the next non-empty line does not look like a header. If the
                // buffer does not yet contain the full next line, ask the caller to
                // wait for more bytes before deciding, otherwise TCP framing can
                // terminate the header block too early.
                if profile_has(
                    self.profile.as_ref(),
                    CompatibilityCapability::HeaderNormalization,
                ) && self.looks_like_header_after_blank(consumed)
                {
                    consumed += 2;
                    continue;
                }
                // End of headers
                consumed += 2;
                let content_length = match headers.get(&HeaderName::ContentLength) {
                    Some(value) => {
                        let trimmed = value.as_str().trim();
                        if trimmed.is_empty() {
                            return Some(Err(SipError::new(
                                SipErrorKind::InvalidHeader,
                                Some(consumed),
                                "Content-Length header is empty",
                            )));
                        }
                        match trimmed.parse::<usize>() {
                            Ok(n) => n,
                            Err(_) => {
                                return Some(Err(SipError::new(
                                    SipErrorKind::InvalidHeader,
                                    Some(consumed),
                                    "Content-Length header is not a non-negative integer",
                                )));
                            }
                        }
                    }
                    None => {
                        if !self.config.datagram_mode {
                            return Some(Err(SipError::new(
                                SipErrorKind::InvalidFraming,
                                Some(consumed),
                                "Content-Length is required for stream framing",
                            )));
                        }
                        let inferred = self.buffer.len().saturating_sub(consumed);
                        headers.append(
                            HeaderName::ContentLength,
                            HeaderValue::new(inferred.to_string()),
                        );
                        inferred
                    }
                };
                if content_length > self.config.max_body_bytes {
                    return Some(Err(SipError::new(
                        SipErrorKind::BodyTooLarge,
                        Some(consumed),
                        "Content-Length exceeds body limit",
                    )));
                }
                self.buffer.drain(..consumed);
                return Some(Ok((headers, content_length, consumed)));
            }

            if let Some(end) = self.find_crlf(consumed) {
                let line_bytes = end - consumed;
                if line_bytes > self.config.max_header_line_bytes {
                    return Some(Err(SipError::new(
                        SipErrorKind::HeaderTooLong,
                        Some(consumed),
                        "header line exceeds limit",
                    )));
                }
                let line = match std::str::from_utf8(&self.buffer[consumed..end]) {
                    Ok(s) => s,
                    Err(_) => {
                        return Some(Err(SipError::new(
                            SipErrorKind::InvalidHeader,
                            Some(consumed),
                            "header is not valid UTF-8",
                        )));
                    }
                };

                // Handle continuation lines (obs-fold); forbidden by RFC but reject safely.
                if line.starts_with(' ') || line.starts_with('\t') {
                    return Some(Err(SipError::new(
                        SipErrorKind::InvalidHeader,
                        Some(consumed),
                        "header continuation not supported",
                    )));
                }

                let (name, value) = match line.split_once(':') {
                    Some(pair) => pair,
                    None => {
                        return Some(Err(SipError::new(
                            SipErrorKind::InvalidHeader,
                            Some(consumed),
                            "missing colon",
                        )));
                    }
                };
                let name = HeaderName::parse(name.trim());
                let mut value = HeaderValue::new(value.trim());
                if name == HeaderName::CSeq
                    && profile_has(
                        self.profile.as_ref(),
                        CompatibilityCapability::HeaderNormalization,
                    )
                {
                    value = normalize_cseq_value(value);
                }
                headers.append(name, value);

                header_count += 1;
                let line_len_with_crlf = end - consumed + 2;
                header_block_size += line_len_with_crlf;
                consumed += line_len_with_crlf;
            } else {
                return None;
            }
        }
    }

    fn find_crlf(&self, start: usize) -> Option<usize> {
        (start..self.buffer.len().saturating_sub(1))
            .find(|&i| self.buffer[i] == b'\r' && self.buffer.get(i + 1) == Some(&b'\n'))
    }

    /// Returns true if the line immediately after a blank line (at `consumed`)
    /// looks like a SIP header (non-empty token, a colon, and no leading
    /// whitespace). Used by `HeaderNormalization` to skip intra-header blank
    /// lines without treating them as the body separator.
    ///
    /// Only applies the intra-header blank heuristic when there is a complete
    /// next line already buffered. If there are no bytes after the blank line, or
    /// the next line is incomplete, the blank line is treated as the header
    /// terminator. This avoids deadlocking stream parsers on body-less messages
    /// (e.g. `REGISTER`) or bodies that do not end with a CRLF.
    fn looks_like_header_after_blank(&self, consumed: usize) -> bool {
        let start = consumed + 2; // skip the leading CRLF
        if start >= self.buffer.len() {
            return false;
        }
        let Some(end) = self.find_crlf(start) else {
            return false;
        };
        let line = &self.buffer[start..end];
        // Must not be empty or continuation.
        if line.is_empty() || line[0].is_ascii_whitespace() {
            return false;
        }
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            return false;
        };
        let name = &line[..colon];
        if name.is_empty() {
            return false;
        }
        // Header name must be a single token (no spaces or non-printable chars).
        name.iter()
            .all(|&b| b.is_ascii_alphanumeric() || b"-._!%$*&+^`{|}~".contains(&b))
    }
}

fn profile_has(profile: Option<&CompatibilityProfile>, cap: CompatibilityCapability) -> bool {
    profile.is_some_and(|p| p.has(cap))
}

fn normalize_cseq_value(value: HeaderValue) -> HeaderValue {
    let raw = value.as_str();
    let trimmed = raw.trim();
    let (num, method) = match trimmed.split_once(char::is_whitespace) {
        Some((num, method)) => (num, method.trim_start()),
        None => return value,
    };
    HeaderValue::new(format!("{} {}", num, method.to_ascii_uppercase()))
}

fn build_message(
    start_line: &str,
    headers: &SipHeaders,
    body: Body,
    content_length: usize,
    profile: Option<&CompatibilityProfile>,
) -> Result<SipMessage, SipError> {
    if body.len() != content_length {
        return Err(SipError::new(
            SipErrorKind::ContentLengthMismatch,
            None,
            format!(
                "Content-Length {content_length} != body length {}",
                body.len()
            ),
        ));
    }

    if let Some(rest) = start_line.strip_prefix("SIP/2.0 ") {
        let mut parts = rest.splitn(2, ' ');
        let code: u16 = parts
            .next()
            .ok_or_else(|| SipError::new(SipErrorKind::InvalidStartLine, None, "missing code"))?
            .parse()
            .map_err(|_| SipError::new(SipErrorKind::InvalidStartLine, None, "invalid code"))?;
        let reason = parts.next().unwrap_or("").to_string();
        Ok(SipMessage::Response {
            line: StatusLine::new(code, reason),
            headers: headers.clone(),
            body,
        })
    } else {
        let mut parts = start_line.splitn(3, ' ');
        let method_str = parts
            .next()
            .ok_or_else(|| SipError::new(SipErrorKind::InvalidStartLine, None, "missing method"))?;
        let uri_str = parts
            .next()
            .ok_or_else(|| SipError::new(SipErrorKind::InvalidStartLine, None, "missing URI"))?;
        let _version = parts.next().ok_or_else(|| {
            SipError::new(SipErrorKind::InvalidStartLine, None, "missing version")
        })?;
        let method_str = if profile_has(profile, CompatibilityCapability::HeaderNormalization) {
            method_str.to_ascii_uppercase()
        } else {
            method_str.to_string()
        };
        let method = Method::parse(&method_str)?;
        let uri = SipUri::parse(uri_str)?;
        Ok(SipMessage::Request {
            line: RequestLine::new(method, uri),
            headers: headers.clone(),
            body,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn datagram(data: &str) -> Result<SipMessage, SipError> {
        SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default())
    }

    #[test]
    fn compact_headers_parse_to_canonical_names() {
        let msg = datagram(
            "REGISTER sip:registrar SIP/2.0\r\n\
             v: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKabc\r\n\
             f: <sip:a@example.com>;tag=1\r\n\
             t: <sip:a@example.com>\r\n\
             i: call-1@example.com\r\n\
             CSeq: 1 REGISTER\r\n\
             l: 0\r\n\r\n",
        )
        .expect("parse compact headers");
        let SipMessage::Request { headers, .. } = &msg else {
            panic!("expected request");
        };
        assert!(headers.get(&HeaderName::Via).is_some());
        assert!(headers.get(&HeaderName::From).is_some());
        assert!(headers.get(&HeaderName::To).is_some());
        assert!(headers.get(&HeaderName::CallId).is_some());
        assert_eq!(msg.call_id(), Some("call-1@example.com"));
    }

    #[test]
    fn unknown_headers_are_preserved_as_other() {
        let msg = datagram(
            "REGISTER sip:registrar SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKabc\r\n\
             X-Vendor-Tag: keep-me\r\n\
             CSeq: 1 REGISTER\r\n\
             Content-Length: 0\r\n\r\n",
        )
        .expect("parse unknown header");
        let SipMessage::Request { headers, .. } = &msg else {
            panic!("expected request");
        };
        let value = headers.get(&HeaderName::Other("x-vendor-tag".to_string()));
        assert_eq!(value.map(|v| v.as_str()), Some("keep-me"));
    }

    #[test]
    fn unknown_headers_count_against_the_bound() {
        let mut header_lines = String::new();
        for i in 0..10 {
            header_lines.push_str(&format!("X-Ext-{i}: v\r\n"));
        }
        let raw = format!(
            "REGISTER sip:registrar SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKabc\r\n\
             {header_lines}\
             CSeq: 1 REGISTER\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let config = SipParserConfig {
            max_headers: 4,
            datagram_mode: true,
            ..SipParserConfig::default()
        };
        let err = SipParser::parse_datagram(raw.as_bytes(), config)
            .expect_err("too many headers must be rejected");
        assert_eq!(err.kind, SipErrorKind::TooManyHeaders);
    }

    #[test]
    fn lower_case_method_rejected_without_profile() {
        let raw = "register sip:registrar SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKabc\r\n\
                   CSeq: 1 register\r\n\
                   Content-Length: 0\r\n\r\n";
        assert!(SipParser::parse_datagram(raw.as_bytes(), SipParserConfig::default()).is_err());
    }

    #[test]
    fn header_normalization_uppercases_method_and_cseq() {
        let profile = CompatibilityProfile {
            capabilities: vec![CompatibilityCapability::HeaderNormalization],
            ..Default::default()
        };
        let raw = "register sip:registrar SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKabc\r\n\
                   CSeq: 1 register\r\n\
                   Content-Length: 0\r\n\r\n";
        let msg = SipParser::parse_datagram_with_profile(
            raw.as_bytes(),
            SipParserConfig::default(),
            Some(&profile),
        )
        .expect("normalize request line and cseq");
        let SipMessage::Request { line, headers, .. } = msg else {
            panic!("expected request");
        };
        assert_eq!(line.method, Method::Register);
        assert_eq!(
            headers.get(&HeaderName::CSeq).unwrap().as_str(),
            "1 REGISTER"
        );
    }

    #[test]
    fn header_normalization_skips_intra_header_blank_line() {
        let profile = CompatibilityProfile {
            capabilities: vec![CompatibilityCapability::HeaderNormalization],
            ..Default::default()
        };
        let body_bytes = br#"<?xml version="1.0"?><Notify><CmdType>Keepalive</CmdType><SN>1</SN><DeviceID>34020000001320000001</DeviceID><Status>OK</Status></Notify>"#;
        let raw = format!(
            "MESSAGE sip:34020000002000000001@example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKabc\r\n\
From: <sip:a@example.com>;tag=1\r\n\
To: <sip:a@example.com>\r\n\
Call-ID: call-1@example.com\r\n\
CSeq: 1 message\r\n\
\r\n\
Content-Type: application/manscdp+xml\r\n\
Content-Length: {}\r\n\r\n",
            body_bytes.len()
        );
        let mut raw = raw.into_bytes();
        raw.extend_from_slice(body_bytes);
        let msg = SipParser::parse_datagram_with_profile(
            &raw,
            SipParserConfig::default(),
            Some(&profile),
        )
        .expect("skip intra-header blank line");
        let SipMessage::Request {
            line,
            headers,
            body,
        } = msg
        else {
            panic!("expected request");
        };
        assert_eq!(line.method, Method::Message);
        assert_eq!(
            headers.get(&HeaderName::CSeq).unwrap().as_str(),
            "1 MESSAGE"
        );
        assert_eq!(
            headers.get(&HeaderName::ContentType).unwrap().as_str(),
            "application/manscdp+xml"
        );
        assert_eq!(body, body_bytes);
    }

    #[test]
    fn header_normalization_bodyless_register_in_stream_mode() {
        let profile = CompatibilityProfile {
            capabilities: vec![CompatibilityCapability::HeaderNormalization],
            ..Default::default()
        };
        let raw = "register sip:registrar SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKabc\r\n\
                   CSeq: 1 register\r\n\
                   Content-Length: 0\r\n\r\n";
        let mut parser = SipParser::new_with_profile(SipParserConfig::default(), Some(profile));
        parser.feed(raw.as_bytes()).unwrap();
        let msg = parser
            .pop_message()
            .expect("stream parser should parse a body-less REGISTER without trailing bytes")
            .expect("valid request");
        let SipMessage::Request { line, body, .. } = msg else {
            panic!("expected request");
        };
        assert_eq!(line.method, Method::Register);
        assert!(body.is_empty());
    }

    #[test]
    fn header_normalization_body_without_trailing_crlf_in_stream_mode() {
        let profile = CompatibilityProfile {
            capabilities: vec![CompatibilityCapability::HeaderNormalization],
            ..Default::default()
        };
        let body_bytes = br#"<?xml version="1.0"?><Notify><CmdType>Keepalive</CmdType></Notify>"#;
        let raw = format!(
            "MESSAGE sip:target@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKabc\r\n\
             From: <sip:a@example.com>;tag=1\r\n\
             To: <sip:a@example.com>\r\n\
             Call-ID: call-1@example.com\r\n\
             CSeq: 1 message\r\n\
             Content-Type: application/manscdp+xml\r\n\
             Content-Length: {}\r\n\r\n",
            body_bytes.len()
        );
        let mut raw = raw.into_bytes();
        raw.extend_from_slice(body_bytes);
        let mut parser = SipParser::new_with_profile(SipParserConfig::default(), Some(profile));
        parser.feed(&raw).unwrap();
        let msg = parser
            .pop_message()
            .expect("stream parser should parse a body that does not end with CRLF")
            .expect("valid request");
        let SipMessage::Request { body, .. } = msg else {
            panic!("expected request");
        };
        assert_eq!(body, body_bytes);
    }
}

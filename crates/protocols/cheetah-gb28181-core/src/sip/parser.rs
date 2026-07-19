//! Sans-I/O SIP parser for datagram and stream transports.

use super::error::{SipError, SipErrorKind};
use super::headers::{HeaderName, HeaderValue, SipHeaders};
use super::message::{Body, Method, RequestLine, SipMessage, StatusLine};
use super::uri::SipUri;

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
    state: ParserState,
    buffer: Vec<u8>,
}

impl SipParser {
    /// Creates a parser with the given limits.
    pub fn new(config: SipParserConfig) -> Self {
        Self {
            config,
            state: ParserState::default(),
            buffer: Vec::new(),
        }
    }

    /// Parses a complete UDP datagram, returning the message and any trailing bytes.
    ///
    /// # Errors
    ///
    /// Returns `SipError` for malformed or oversized messages.
    pub fn parse_datagram(
        data: &[u8],
        mut config: SipParserConfig,
    ) -> Result<SipMessage, SipError> {
        config.datagram_mode = true;
        let mut parser = Self::new(config);
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
                let result = build_message(&start_line, &headers, body, content_length);
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
                let value = HeaderValue::new(value.trim());
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
}

fn build_message(
    start_line: &str,
    headers: &SipHeaders,
    body: Body,
    content_length: usize,
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
        let method = Method::parse(method_str)?;
        let uri = SipUri::parse(uri_str)?;
        Ok(SipMessage::Request {
            line: RequestLine::new(method, uri),
            headers: headers.clone(),
            body,
        })
    }
}

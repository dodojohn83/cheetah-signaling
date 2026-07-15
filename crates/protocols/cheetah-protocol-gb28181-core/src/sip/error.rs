//! SIP parsing and encoding errors.

use std::fmt;

/// Reason for a SIP message error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SipErrorKind {
    /// First line exceeded the configured limit.
    StartLineTooLong,
    /// A header name or value exceeded the configured limit.
    HeaderTooLong,
    /// Too many headers.
    TooManyHeaders,
    /// Total header block exceeded the configured limit.
    HeadersTooLarge,
    /// Body exceeded the configured limit.
    BodyTooLarge,
    /// Parser buffer exceeded the configured limit.
    BufferTooLarge,
    /// Content-Length conflicted with actual body length.
    ContentLengthMismatch,
    /// Malformed start line.
    InvalidStartLine,
    /// Malformed header line.
    InvalidHeader,
    /// Malformed SIP URI.
    InvalidUri,
    /// Required header missing.
    MissingRequiredHeader,
    /// Line termination or framing error.
    InvalidFraming,
}

/// Errors returned by the Sans-I/O SIP parser and encoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SipError {
    /// Class of the error.
    pub kind: SipErrorKind,
    /// Byte offset into the input where the error was detected, when known.
    pub offset: Option<usize>,
    /// Human-readable diagnostic; must not contain credentials or bodies.
    pub message: String,
}

impl SipError {
    /// Creates a new SIP error.
    pub fn new(kind: SipErrorKind, offset: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            kind,
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for SipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.kind)?;
        if let Some(offset) = self.offset {
            write!(f, " at offset {offset}")?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for SipError {}

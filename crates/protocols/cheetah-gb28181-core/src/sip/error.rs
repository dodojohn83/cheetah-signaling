//! SIP protocol errors.

use std::fmt;

/// Maximum byte length of a `SipError` diagnostic message.
const MAX_MESSAGE_BYTES: usize = 1024;

/// Truncates `s` to at most `max` bytes, never splitting a multi-byte character.
fn clamp_string_bytes(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

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
    /// Transaction layer timeout.
    TransactionTimeout,
    /// Transaction was cancelled by a matching CANCEL request.
    TransactionCancelled,
    /// Transport reported an unrecoverable failure.
    TransportFailure,
    /// Digest authentication failed.
    AuthenticationFailure,
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
    ///
    /// The diagnostic message is clamped to [`MAX_MESSAGE_BYTES`] so upstream
    /// errors cannot inflate logs or downstream problem details.
    pub fn new(kind: SipErrorKind, offset: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            kind,
            offset,
            message: clamp_string_bytes(message.into(), MAX_MESSAGE_BYTES),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_is_clamped_to_max_bytes() {
        let huge = "x".repeat(2000);
        let err = SipError::new(SipErrorKind::InvalidStartLine, Some(0), huge);
        assert!(err.message.len() <= MAX_MESSAGE_BYTES);
    }

    #[test]
    fn clamp_preserves_utf8_boundary() {
        let s = "α".repeat(600);
        let err = SipError::new(SipErrorKind::InvalidHeader, None, s);
        assert!(err.message.is_char_boundary(err.message.len()));
        assert!(err.message.len() <= MAX_MESSAGE_BYTES);
    }
}

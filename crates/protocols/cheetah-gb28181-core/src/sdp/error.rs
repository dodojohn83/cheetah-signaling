//! SDP parsing and validation errors.

/// Maximum byte length of the human-readable message carried by an `SdpError`.
const MAX_SDP_ERROR_BYTES: usize = 1024;

/// Truncates `message` at a UTF-8 character boundary so it is at most
/// `MAX_SDP_ERROR_BYTES` long.
fn clamp_message(message: impl std::fmt::Display) -> String {
    let s = message.to_string();
    if s.len() <= MAX_SDP_ERROR_BYTES {
        return s;
    }
    let mut idx = MAX_SDP_ERROR_BYTES;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    s[..idx].to_string()
}

/// Errors that can occur when parsing or encoding SDP.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SdpError {
    /// The SDP body exceeds a configured size or length limit.
    #[error("SDP size or length limit exceeded: {0}")]
    LimitExceeded(String),
    /// A required SDP line is missing or malformed.
    #[error("malformed SDP: {0}")]
    Malformed(String),
    /// An SDP value is not supported in this implementation.
    #[error("unsupported SDP value: {0}")]
    Unsupported(String),
}

impl SdpError {
    /// Creates a `LimitExceeded` error with a clamped message.
    pub fn limit_exceeded(message: impl std::fmt::Display) -> Self {
        Self::LimitExceeded(clamp_message(message))
    }

    /// Creates a `Malformed` error with a clamped message.
    pub fn malformed(message: impl std::fmt::Display) -> Self {
        Self::Malformed(clamp_message(message))
    }

    /// Creates an `Unsupported` error with a clamped message.
    pub fn unsupported(message: impl std::fmt::Display) -> Self {
        Self::Unsupported(clamp_message(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_passthrough_short_messages() {
        let err = SdpError::malformed("short");
        assert_eq!(err.to_string(), "malformed SDP: short");

        let err = SdpError::limit_exceeded("size");
        assert_eq!(err.to_string(), "SDP size or length limit exceeded: size");

        let err = SdpError::unsupported("codec");
        assert_eq!(err.to_string(), "unsupported SDP value: codec");
    }

    #[test]
    fn constructors_clamp_long_messages_at_char_boundary() {
        let padding = "x".repeat(MAX_SDP_ERROR_BYTES);
        let trailer = "\u{1F600}"; // 4-byte UTF-8
        let message = format!("{padding}{trailer}");

        let err = SdpError::malformed(message.clone());
        let inner = match err {
            SdpError::Malformed(s) => s,
            _ => panic!("expected Malformed"),
        };
        assert!(inner.len() <= MAX_SDP_ERROR_BYTES);
        assert!(inner.is_char_boundary(inner.len()));
        assert!(message.starts_with(&inner));
    }

    #[test]
    fn clamp_avoids_splitting_multi_byte_char() {
        // Create a string where byte index MAX_SDP_ERROR_BYTES lands in the middle
        // of a 4-byte emoji. The result must end on a boundary.
        let base = "a".repeat(MAX_SDP_ERROR_BYTES - 2);
        let message = format!("{base}\u{1F600}\u{1F600}");
        let err = SdpError::unsupported(message);
        let inner = match err {
            SdpError::Unsupported(s) => s,
            _ => panic!("expected Unsupported"),
        };
        assert_eq!(inner.len(), MAX_SDP_ERROR_BYTES - 2);
    }
}

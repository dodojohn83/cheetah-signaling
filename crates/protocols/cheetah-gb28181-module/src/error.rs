//! Errors returned by the GB28181 access module.

/// Maximum byte length of the human-readable message carried by an `AccessError`.
const MAX_ACCESS_ERROR_BYTES: usize = 1024;

/// Truncates `message` at a UTF-8 character boundary so it is at most
/// `MAX_ACCESS_ERROR_BYTES` long.
fn clamp_message(message: impl std::fmt::Display) -> String {
    let s = message.to_string();
    if s.len() <= MAX_ACCESS_ERROR_BYTES {
        return s;
    }
    let mut idx = MAX_ACCESS_ERROR_BYTES;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    s[..idx].to_string()
}

/// Errors produced while processing GB28181 messages.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum AccessError {
    #[error("no domain configuration matches the request URI")]
    UnknownDomain,
    #[error("request method is not supported for this endpoint")]
    UnsupportedMethod,
    #[error("unsupported command type: {0}")]
    UnsupportedCmdType(String),
    #[error("missing or invalid device identifier")]
    InvalidDeviceId,
    #[error("invalid domain identifier")]
    InvalidDomainId,
    #[error("missing or malformed Contact header")]
    InvalidContact,
    #[error("missing or malformed Expires value")]
    InvalidExpires,
    #[error("authentication required")]
    AuthenticationRequired,
    #[error("authentication failed")]
    AuthenticationFailed,
    #[error("device is not registered or session has expired")]
    NotRegistered,
    #[error("registration table is full")]
    RegistrationTableFull,
    #[error("malformed or disallowed XML body: {0}")]
    InvalidXml(String),
    #[error("unsupported content type: {0}")]
    UnsupportedContentType(String),
    #[error("internal module error: {0}")]
    Internal(String),
}

impl AccessError {
    /// Creates an `UnsupportedCmdType` error with a clamped message.
    pub fn unsupported_cmd_type(message: impl std::fmt::Display) -> Self {
        Self::UnsupportedCmdType(clamp_message(message))
    }

    /// Creates an `InvalidXml` error with a clamped message.
    pub fn invalid_xml(message: impl std::fmt::Display) -> Self {
        Self::InvalidXml(clamp_message(message))
    }

    /// Creates an `UnsupportedContentType` error with a clamped message.
    pub fn unsupported_content_type(message: impl std::fmt::Display) -> Self {
        Self::UnsupportedContentType(clamp_message(message))
    }

    /// Creates an `Internal` error with a clamped message.
    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::Internal(clamp_message(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_passthrough_short_messages() {
        assert_eq!(
            AccessError::invalid_xml("short").to_string(),
            "malformed or disallowed XML body: short"
        );
        assert_eq!(
            AccessError::unsupported_content_type("text/plain").to_string(),
            "unsupported content type: text/plain"
        );
        assert_eq!(
            AccessError::unsupported_cmd_type("PTZ").to_string(),
            "unsupported command type: PTZ"
        );
        assert_eq!(
            AccessError::internal("oops").to_string(),
            "internal module error: oops"
        );
    }

    #[test]
    fn constructors_clamp_long_messages_at_char_boundary() {
        let padding = "x".repeat(MAX_ACCESS_ERROR_BYTES);
        let trailer = "\u{1F600}";
        let message = format!("{padding}{trailer}");
        let err = AccessError::invalid_xml(message.clone());
        let inner = match err {
            AccessError::InvalidXml(s) => s,
            _ => panic!("expected InvalidXml"),
        };
        assert!(inner.len() <= MAX_ACCESS_ERROR_BYTES);
        assert!(inner.is_char_boundary(inner.len()));
        assert!(message.starts_with(&inner));
    }
}

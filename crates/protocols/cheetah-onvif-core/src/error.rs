//! ONVIF core error types.

/// Maximum byte length of an `OnvifError` diagnostic message.
const MAX_ONVIF_ERROR_BYTES: usize = 1024;

/// Short alias for `Result<T, OnvifError>`.
pub type OnvifResult<T> = Result<T, OnvifError>;

/// Truncates `message` to a UTF-8-safe bounded length for diagnostics.
fn clamp_message(message: impl std::fmt::Display) -> String {
    let s = message.to_string();
    if s.len() <= MAX_ONVIF_ERROR_BYTES {
        return s;
    }
    let mut end = MAX_ONVIF_ERROR_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Errors produced by ONVIF core parsers and builders.
#[derive(Debug, thiserror::Error)]
pub enum OnvifError {
    /// XML encoding/decoding error from the underlying parser.
    #[error("xml error: {0}")]
    Xml(String),

    /// A required element is missing.
    #[error("missing field: {0}")]
    MissingField(String),

    /// A field is present but has an out-of-range or otherwise invalid value.
    #[error("invalid field: {0}")]
    InvalidField(String),

    /// The supplied URL failed validation.
    #[error("invalid xaddr: {0}")]
    InvalidXAddr(String),

    /// Security parameter error.
    #[error("security error: {0}")]
    Security(String),

    /// SSRF policy rejected a supplied address.
    #[error("ssrf rejected: {0}")]
    SsrfRejected(String),

    /// A value exceeded a configured limit.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),

    /// Underlying I/O error while writing XML.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl OnvifError {
    /// Creates an `Xml` error with a bounded diagnostic message.
    pub fn xml(message: impl std::fmt::Display) -> Self {
        Self::Xml(clamp_message(message))
    }

    /// Creates a `MissingField` error with a bounded field name.
    pub fn missing_field(field: impl std::fmt::Display) -> Self {
        Self::MissingField(clamp_message(field))
    }

    /// Creates an `InvalidField` error with a bounded diagnostic message.
    pub fn invalid_field(message: impl std::fmt::Display) -> Self {
        Self::InvalidField(clamp_message(message))
    }

    /// Creates an `InvalidXAddr` error with a bounded diagnostic message.
    pub fn invalid_xaddr(message: impl std::fmt::Display) -> Self {
        Self::InvalidXAddr(clamp_message(message))
    }

    /// Creates a `Security` error with a bounded diagnostic message.
    pub fn security(message: impl std::fmt::Display) -> Self {
        Self::Security(clamp_message(message))
    }

    /// Creates an `SsrfRejected` error with a bounded diagnostic message.
    pub fn ssrf_rejected(message: impl std::fmt::Display) -> Self {
        Self::SsrfRejected(clamp_message(message))
    }

    /// Creates a `LimitExceeded` error with a bounded diagnostic message.
    pub fn limit_exceeded(message: impl std::fmt::Display) -> Self {
        Self::LimitExceeded(clamp_message(message))
    }
}

impl From<quick_xml::Error> for OnvifError {
    fn from(value: quick_xml::Error) -> Self {
        Self::xml(value)
    }
}

impl From<url::ParseError> for OnvifError {
    fn from(value: url::ParseError) -> Self {
        Self::invalid_xaddr(value)
    }
}

impl From<std::string::FromUtf8Error> for OnvifError {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Self::xml(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_is_clamped_to_max_bytes() {
        let long = "x".repeat(MAX_ONVIF_ERROR_BYTES + 100);
        let err = OnvifError::invalid_field(format!("prefix: {long}"));
        assert!(err.to_string().len() <= MAX_ONVIF_ERROR_BYTES + "invalid field: ".len());
        // The clamped payload itself must not exceed the configured limit.
        if let OnvifError::InvalidField(msg) = err {
            assert_eq!(msg.len(), MAX_ONVIF_ERROR_BYTES);
        } else {
            panic!("expected InvalidField variant");
        }
    }

    #[test]
    fn clamp_respects_utf8_char_boundaries() {
        let text = "x".repeat(MAX_ONVIF_ERROR_BYTES - 1) + "é";
        let err = OnvifError::xml(text);
        if let OnvifError::Xml(msg) = err {
            assert!(msg.len() <= MAX_ONVIF_ERROR_BYTES);
            assert!(msg.is_char_boundary(msg.len()));
        } else {
            panic!("expected Xml variant");
        }
    }
}

//! ONVIF core error types.

/// Short alias for `Result<T, OnvifError>`.
pub type OnvifResult<T> = Result<T, OnvifError>;

/// Errors produced by ONVIF core parsers and builders.
#[derive(Debug, thiserror::Error)]
pub enum OnvifError {
    /// XML encoding/decoding error from the underlying parser.
    #[error("xml error: {0}")]
    Xml(String),

    /// A required element or attribute is missing or malformed.
    #[error("missing or malformed field: {0}")]
    MissingField(String),

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

impl From<quick_xml::Error> for OnvifError {
    fn from(value: quick_xml::Error) -> Self {
        Self::Xml(value.to_string())
    }
}

impl From<url::ParseError> for OnvifError {
    fn from(value: url::ParseError) -> Self {
        Self::InvalidXAddr(value.to_string())
    }
}

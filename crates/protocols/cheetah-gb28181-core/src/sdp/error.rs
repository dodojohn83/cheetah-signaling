//! SDP parsing and validation errors.

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

//! Errors returned by the GB28181 downstream platform access module.

use crate::error::AccessError;

/// Errors produced while processing lower-platform GB28181 messages.
#[derive(Debug, thiserror::Error)]
pub enum DownstreamError {
    /// A generic access-module error propagated from shared helpers.
    #[error("access error: {0}")]
    Access(#[from] AccessError),
    /// The requested platform link is not registered or has expired.
    #[error("platform link is not registered")]
    NotRegistered,
    /// The supplied downstream command is not supported.
    #[error("unsupported downstream command")]
    UnsupportedCommand,
    /// A requested outbound operation requires a registered contact address.
    #[error("platform link has no usable contact URI")]
    NoContact,
    /// The outbound request could not be encoded.
    #[error("failed to encode downstream request: {0}")]
    Encode(String),
    /// A security-relevant token contained forbidden line breaks.
    #[error("token contains forbidden line breaks")]
    InvalidToken,
}

//! Errors returned by the GB28181 access module.

/// Errors produced while processing GB28181 messages.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum AccessError {
    #[error("no domain configuration matches the request URI")]
    UnknownDomain,
    #[error("request method is not supported for this endpoint")]
    UnsupportedMethod,
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
    #[error("internal module error: {0}")]
    Internal(String),
}

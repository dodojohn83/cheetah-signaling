//! Ports used by the GB28181 module to query external state without depending
//! on concrete infrastructure.

use crate::types::DeviceId;
use cheetah_signal_types::clamp_str;
use secrecy::SecretString;

/// Maximum byte length of a `CredentialError` diagnostic message.
const MAX_CREDENTIAL_ERROR_BYTES: usize = 1024;

/// Error returned by a [`CredentialProvider`] when a password cannot be
/// retrieved because the underlying backend failed.
///
/// `Ok(None)` is the correct way to indicate that the device simply has no
/// configured password.
#[derive(Clone, Debug, thiserror::Error)]
pub enum CredentialError {
    /// The credential backend returned an error.
    #[error("credential backend error: {0}")]
    Backend(String),
}

impl CredentialError {
    /// Creates a `Backend` error with a bounded diagnostic message.
    pub fn backend(message: impl std::fmt::Display) -> Self {
        Self::Backend(clamp_str(&message.to_string(), MAX_CREDENTIAL_ERROR_BYTES))
    }
}

/// Looks up the digest password for a device identifier.
///
/// Implementations are provided by the application/driver layer, which may
/// delegate to a secret store, configuration map, or repository.
pub trait CredentialProvider: Send + Sync {
    /// Returns the configured password for `device_id`, or `Ok(None)` if the
    /// device has no configured password.
    fn password_for(&self, device_id: &DeviceId) -> Result<Option<SecretString>, CredentialError>;
}

impl<F> CredentialProvider for F
where
    F: Fn(&DeviceId) -> Result<Option<SecretString>, CredentialError> + Send + Sync,
{
    fn password_for(&self, device_id: &DeviceId) -> Result<Option<SecretString>, CredentialError> {
        (self)(device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_error_clamps_oversized_message() {
        let long = "x".repeat(2048);
        let err = CredentialError::backend(format!("secret store failure: {long}"));
        let CredentialError::Backend(msg) = err;
        assert_eq!(msg.len(), MAX_CREDENTIAL_ERROR_BYTES);
        assert!(msg.is_char_boundary(msg.len()));
        assert!(msg.starts_with("secret store failure: "));
    }
}

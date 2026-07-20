//! Ports used by the GB28181 module to query external state without depending
//! on concrete infrastructure.

use crate::types::DeviceId;
use secrecy::SecretString;

/// Error returned by a [`CredentialProvider`] when a password cannot be
/// retrieved because the underlying backend failed.
///
/// `Ok(None)` is the correct way to indicate that the device simply has no
/// configured password.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    /// The credential backend returned an error.
    #[error("credential backend error: {0}")]
    Backend(String),
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

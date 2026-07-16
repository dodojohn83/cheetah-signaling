//! Ports used by the GB28181 module to query external state without depending
//! on concrete infrastructure.

use crate::types::DeviceId;
use secrecy::SecretString;

/// Looks up the digest password for a device identifier.
///
/// Implementations are provided by the application/driver layer, which may
/// delegate to a secret store, configuration map, or repository.
pub trait CredentialProvider: Send + Sync {
    /// Returns the configured password for `device_id`, if known.
    fn password_for(&self, device_id: &DeviceId) -> Option<SecretString>;
}

impl<F> CredentialProvider for F
where
    F: Fn(&DeviceId) -> Option<SecretString> + Send + Sync,
{
    fn password_for(&self, device_id: &DeviceId) -> Option<SecretString> {
        (self)(device_id)
    }
}

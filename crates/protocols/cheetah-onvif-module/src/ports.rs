//! Ports used by the ONVIF module to query external state without depending on
//! concrete infrastructure.

use crate::types::DeviceInformation;
use cheetah_signal_types::{DeviceId, TenantId};
use secrecy::SecretString;

/// Looks up credentials for an ONVIF device.
///
/// Implementations are provided by the application/driver layer.
pub trait CredentialProvider: Send + Sync {
    /// Returns the configured username for `device_id`, if known.
    fn username_for(&self, device_id: &DeviceId) -> Option<String>;
    /// Returns the configured password for `device_id`, if known.
    fn password_for(&self, device_id: &DeviceId) -> Option<SecretString>;
    /// Returns the WSSE nonce size in bytes for `device_id`.
    fn nonce_size_bytes(&self, _device_id: &DeviceId) -> usize {
        16
    }
}

impl<F, G> CredentialProvider for (F, G)
where
    F: Fn(&DeviceId) -> Option<String> + Send + Sync,
    G: Fn(&DeviceId) -> Option<SecretString> + Send + Sync,
{
    fn username_for(&self, device_id: &DeviceId) -> Option<String> {
        (self.0)(device_id)
    }
    fn password_for(&self, device_id: &DeviceId) -> Option<SecretString> {
        (self.1)(device_id)
    }
}

/// State queried by the provisioning workflow.
pub trait DeviceStatePort: Send + Sync {
    /// Returns the current provisioning stage for a device.
    fn stage(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Option<crate::types::ProvisioningStage>;
    /// Records parsed device information.
    fn record_device_info(&self, tenant_id: TenantId, device_id: DeviceId, info: DeviceInformation);
}

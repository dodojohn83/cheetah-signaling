//! Stable key and identifier types used by the runtime.

use cheetah_signal_types::{DeviceId, ProtocolSessionId, TenantId};

/// Identifies a device within a tenant.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DeviceKey {
    tenant_id: TenantId,
    device_id: DeviceId,
}

impl DeviceKey {
    /// Creates a new device key.
    pub fn new(tenant_id: TenantId, device_id: DeviceId) -> Self {
        Self {
            tenant_id,
            device_id,
        }
    }

    /// Returns the tenant identifier.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Returns the device identifier.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }
}

/// Identifies a protocol session within a tenant.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SessionKey {
    tenant_id: TenantId,
    protocol_session_id: ProtocolSessionId,
}

impl SessionKey {
    /// Creates a new session key.
    pub fn new(tenant_id: TenantId, protocol_session_id: ProtocolSessionId) -> Self {
        Self {
            tenant_id,
            protocol_session_id,
        }
    }

    /// Returns the tenant identifier.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Returns the protocol session identifier.
    pub fn protocol_session_id(&self) -> ProtocolSessionId {
        self.protocol_session_id
    }
}

/// Identifies a scheduled timer.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TimerId(u64);

impl TimerId {
    /// Creates a new timer identifier from a raw value.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw identifier value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_key_components_round_trip() {
        let tenant_id = TenantId::generate();
        let device_id = DeviceId::generate();
        let key = DeviceKey::new(tenant_id, device_id);
        assert_eq!(key.tenant_id(), tenant_id);
        assert_eq!(key.device_id(), device_id);
    }

    #[test]
    fn session_key_components_round_trip() {
        let tenant_id = TenantId::generate();
        let session_id = ProtocolSessionId::generate();
        let key = SessionKey::new(tenant_id, session_id);
        assert_eq!(key.tenant_id(), tenant_id);
        assert_eq!(key.protocol_session_id(), session_id);
    }

    #[test]
    fn timer_id_exposes_raw_value() {
        let id = TimerId::new(42);
        assert_eq!(id.as_u64(), 42);
    }
}

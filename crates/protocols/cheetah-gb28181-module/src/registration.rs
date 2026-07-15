//! In-memory registration table for one GB28181 domain.

use crate::types::DeviceId;
use std::collections::HashMap;
use std::net::SocketAddr;

/// State for a currently registered GB28181 device.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct Registration {
    /// Source address of the most recent register/keepalive.
    pub source: SocketAddr,
    /// Contact endpoint string returned at registration.
    pub contact: String,
    /// Monotonic time when the registration was created or refreshed.
    pub registered_at: u64,
    /// Granted expiry in seconds.
    pub expires: u32,
    /// Monotonic time of the last keepalive or register.
    pub last_seen: u64,
    /// Whether the device is currently considered offline.
    pub offline: bool,
    /// Raw User-Agent header from registration, if present.
    pub user_agent: Option<String>,
}

/// Simple in-memory registration table keyed by device ID.
#[derive(Clone, Debug, Default)]
pub(crate) struct RegistrationTable {
    table: HashMap<DeviceId, Registration>,
}

impl RegistrationTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    /// Inserts or replaces a registration. Returns the previous registration,
    /// if any.
    pub fn upsert(
        &mut self,
        device_id: DeviceId,
        source: SocketAddr,
        contact: String,
        expires: u32,
        now: u64,
        user_agent: Option<String>,
    ) -> Option<Registration> {
        let registration = Registration {
            source,
            contact,
            registered_at: now,
            expires,
            last_seen: now,
            offline: false,
            user_agent,
        };
        self.table.insert(device_id, registration)
    }

    /// Marks a registered device as still alive. Returns `Some` with the
    /// previous offline flag when the device exists.
    pub fn touch(&mut self, device_id: &DeviceId, source: SocketAddr, now: u64) -> Option<bool> {
        let reg = self.table.get_mut(device_id)?;
        reg.source = source;
        reg.last_seen = now;
        let was_offline = reg.offline;
        reg.offline = false;
        Some(was_offline)
    }

    /// Removes a registration and returns it, if present.
    pub fn remove(&mut self, device_id: &DeviceId) -> Option<Registration> {
        self.table.remove(device_id)
    }

    /// Iterates over all registrations mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&DeviceId, &mut Registration)> {
        self.table.iter_mut()
    }
}

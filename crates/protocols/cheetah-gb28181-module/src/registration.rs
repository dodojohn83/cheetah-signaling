//! In-memory registration table for one GB28181 domain.

use crate::error::AccessError;
use crate::types::DeviceId;
use cheetah_gb28181_core::EndpointRoute;
use std::collections::HashMap;
use std::net::SocketAddr;

/// State for a currently registered GB28181 device.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct Registration {
    /// Typed endpoint route established by the authenticated REGISTER.
    ///
    /// The route (observed source, Via `received`/`rport`, Contact and
    /// advertised endpoint) is only rewritten by another authenticated
    /// REGISTER; keepalive/MESSAGE packets never move it.
    pub route: EndpointRoute,
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
    /// Monotonic sequence number for this registration session, allocated on a
    /// new or recovered registration. Used by downstream consumers such as
    /// bootstrap operation idempotency keys.
    pub registration_sequence: u64,
}

impl Registration {
    /// The authoritative source address for events and downlink routing: the
    /// send target resolved from the endpoint route established at
    /// registration, not the source of the most recent packet.
    pub fn source(&self) -> SocketAddr {
        self.route.send_target()
    }
}

/// Outcome of a keepalive/MESSAGE touch on a registered device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TouchOutcome {
    /// Whether the device was offline before this touch.
    pub was_offline: bool,
    /// Whether the packet arrived from a source that differs from the
    /// established route (a potential source hijack). The stored route is
    /// never changed by a touch regardless of this flag.
    pub source_drift: bool,
}

/// Simple in-memory registration table keyed by device ID.
#[derive(Clone, Debug)]
pub(crate) struct RegistrationTable {
    table: HashMap<DeviceId, Registration>,
    max_registrations: usize,
    next_sequence: u64,
}

impl RegistrationTable {
    pub fn new(max_registrations: usize) -> Self {
        Self {
            table: HashMap::new(),
            max_registrations,
            next_sequence: 0,
        }
    }

    /// Inserts or replaces a registration. Returns the inserted or updated
    /// registration.
    ///
    /// This is only called from an authenticated (or accepted challenge-optional)
    /// REGISTER path, so replacing the endpoint route here is the sanctioned way
    /// to move a device's send route.
    ///
    /// A new registration sequence is allocated when the device was not
    /// previously registered or the previous registration was marked offline,
    /// which gives downstream consumers a stable per-session generation they can
    /// embed in idempotency keys.
    ///
    /// Rejects the insertion if the table is already at capacity and the
    /// device is not already registered. Capacity violations are reported as
    /// `AccessError::RegistrationTableFull`.
    pub fn upsert(
        &mut self,
        device_id: DeviceId,
        route: EndpointRoute,
        contact: String,
        expires: u32,
        now: u64,
        user_agent: Option<String>,
    ) -> Result<Registration, AccessError> {
        let is_new = !self.table.contains_key(&device_id);
        if is_new && self.table.len() >= self.max_registrations {
            return Err(AccessError::RegistrationTableFull);
        }

        let previous = self.table.get(&device_id);
        let previous_sequence = previous.as_ref().and_then(|r| {
            if r.offline {
                None
            } else {
                Some(r.registration_sequence)
            }
        });
        let registration_sequence = if let Some(seq) = previous_sequence {
            seq
        } else {
            self.next_sequence = self.next_sequence.wrapping_add(1);
            self.next_sequence
        };

        let registration = Registration {
            route,
            contact,
            registered_at: now,
            expires,
            last_seen: now,
            offline: false,
            user_agent,
            registration_sequence,
        };
        self.table.insert(device_id, registration.clone());
        Ok(registration)
    }

    /// Marks a registered device as still alive without mutating its endpoint
    /// route.
    ///
    /// The `source` of the keepalive/MESSAGE packet is used only to detect
    /// drift (source hijack); it never overwrites the stored route, which can
    /// only change through an authenticated REGISTER. Returns `None` when the
    /// device is not registered.
    pub fn touch(
        &mut self,
        device_id: &DeviceId,
        source: SocketAddr,
        now: u64,
    ) -> Option<TouchOutcome> {
        let reg = self.table.get_mut(device_id)?;
        reg.last_seen = now;
        let was_offline = reg.offline;
        reg.offline = false;
        let source_drift = reg.route.is_unauthenticated_drift(source);
        Some(TouchOutcome {
            was_offline,
            source_drift,
        })
    }

    /// Removes a registration and returns it, if present.
    pub fn remove(&mut self, device_id: &DeviceId) -> Option<Registration> {
        self.table.remove(device_id)
    }

    /// Returns the resolved send target for a registered device, if any.
    pub fn send_target(&self, device_id: &DeviceId) -> Option<SocketAddr> {
        self.table.get(device_id).map(|reg| reg.route.send_target())
    }

    /// Returns a clone of the endpoint route for a registered device, if any.
    pub fn route(&self, device_id: &DeviceId) -> Option<EndpointRoute> {
        self.table.get(device_id).map(|reg| reg.route.clone())
    }

    /// Iterates over all registrations mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&DeviceId, &mut Registration)> {
        self.table.iter_mut()
    }
}

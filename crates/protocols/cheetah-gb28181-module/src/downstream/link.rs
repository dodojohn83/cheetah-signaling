//! In-memory link table for lower GB28181 platforms.

use crate::error::AccessError;
use crate::types::DeviceId;
use cheetah_gb28181_core::SipUri;
use std::collections::HashMap;
use std::net::SocketAddr;

/// State for a registered lower-platform link.
#[derive(Clone, Debug)]
pub(crate) struct PlatformLink {
    /// Source address observed from the transport.
    pub source: SocketAddr,
    /// Parsed Contact URI for outbound requests.
    pub contact: SipUri,
    /// Monotonic time when the link was created or refreshed.
    pub registered_at: u64,
    /// Granted expiry in seconds.
    pub expires: u32,
    /// Monotonic time of the last keepalive or register.
    pub last_seen: u64,
    /// Whether the link is currently considered offline.
    pub offline: bool,
    /// Call-ID established at registration.
    pub call_id: String,
    /// Local tag used in the `To` header of the 200 OK.
    pub local_tag: String,
    /// Remote tag from the lower platform `From` header.
    pub remote_tag: String,
    /// Next CSeq value for outbound requests.
    pub next_cseq: u32,
}

/// In-memory table keyed by lower-platform identifier.
#[derive(Clone, Debug)]
pub(crate) struct LinkTable {
    table: HashMap<DeviceId, PlatformLink>,
    max_links: usize,
}

impl LinkTable {
    pub fn new(max_links: usize) -> Self {
        Self {
            table: HashMap::new(),
            max_links,
        }
    }

    /// Inserts or replaces a link. Returns the previous link, if any.
    ///
    /// Rejects the insertion if the table is at capacity and the platform is
    /// not already registered.
    pub fn upsert(
        &mut self,
        platform_id: DeviceId,
        link: PlatformLink,
    ) -> Result<Option<PlatformLink>, AccessError> {
        let is_new = !self.table.contains_key(&platform_id);
        if is_new && self.table.len() >= self.max_links {
            return Err(AccessError::RegistrationTableFull);
        }
        Ok(self.table.insert(platform_id, link))
    }

    /// Removes a link and returns it, if present.
    pub fn remove(&mut self, platform_id: &DeviceId) -> Option<PlatformLink> {
        self.table.remove(platform_id)
    }

    /// Returns a mutable reference to a link.
    pub fn get_mut(&mut self, platform_id: &DeviceId) -> Option<&mut PlatformLink> {
        self.table.get_mut(platform_id)
    }

    /// Iterates over all links mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&DeviceId, &mut PlatformLink)> {
        self.table.iter_mut()
    }
}

//! Cluster node model for distributed control plane membership.

use cheetah_signal_types::{DurationMs, NodeId, NodeInstanceId, UtcTimestamp};
use std::collections::HashMap;

/// Capacity limits advertised by a node.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NodeCapacity {
    /// Maximum number of devices this node can own concurrently.
    pub max_devices: u32,
}

/// Current load reported by a node.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NodeLoad {
    /// Number of devices currently owned by this node.
    pub devices: u32,
}

/// A member node of the Cheetah Signaling control plane cluster.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClusterNode {
    /// Stable node identifier.
    pub node_id: NodeId,
    /// Unique incarnation identifier, regenerated on every process start.
    pub instance_id: NodeInstanceId,
    /// Deployment zone or availability zone.
    pub zone: String,
    /// Binary version running on this node.
    pub version: String,
    /// Highest supported contract versions by contract name.
    pub contract_versions: HashMap<String, String>,
    /// Wall-clock time when this instance started.
    pub started_at: UtcTimestamp,
    /// Lease expiration time. The node is considered alive while this is in the future.
    pub lease_until: UtcTimestamp,
    /// Last time the record was changed (heartbeat or registration).
    pub updated_at: UtcTimestamp,
    /// Advertised capacity.
    pub capacity: NodeCapacity,
    /// Latest reported load.
    pub load: NodeLoad,
    /// Whether the node is draining and should not receive new work.
    pub draining: bool,
}

impl ClusterNode {
    /// Creates a new cluster node registration.
    pub fn new(
        node_id: NodeId,
        instance_id: NodeInstanceId,
        zone: impl Into<String>,
        version: impl Into<String>,
        started_at: UtcTimestamp,
    ) -> Self {
        Self {
            node_id,
            instance_id,
            zone: zone.into(),
            version: version.into(),
            contract_versions: HashMap::new(),
            started_at,
            lease_until: started_at,
            updated_at: started_at,
            capacity: NodeCapacity::default(),
            load: NodeLoad::default(),
            draining: false,
        }
    }

    /// Returns true if `now` is before `lease_until`.
    pub fn is_alive(&self, now: UtcTimestamp) -> bool {
        self.lease_until > now
    }

    /// Extends the lease by `duration` from `now`.
    pub fn extend_lease(
        &mut self,
        now: UtcTimestamp,
        duration: DurationMs,
    ) -> Result<(), crate::DomainError> {
        self.lease_until = now
            .checked_add(duration)
            .ok_or_else(|| crate::DomainError::internal("node lease timestamp overflow"))?;
        self.updated_at = now;
        Ok(())
    }
}

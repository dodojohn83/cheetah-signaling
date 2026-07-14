//! Media node model used by the scheduler.

use cheetah_signal_types::{MediaNodeInstanceEpoch, NodeId, UtcTimestamp};
use std::collections::BTreeMap;

/// A single capability advertised by a media node.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaCapability {
    /// Protocol this capability serves, such as `gb28181` or `onvif`.
    pub protocol: String,
    /// Operations supported under this capability, such as `live` or `playback`.
    pub operations: Vec<String>,
    /// Capability constraints, e.g. supported codecs or transports.
    pub constraints: BTreeMap<String, String>,
}

/// Capacity limits advertised by a media node.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaNodeCapacity {
    /// Maximum concurrent sessions.
    pub max_sessions: u64,
    /// Maximum bandwidth in megabits per second.
    pub max_bandwidth_mbps: u64,
    /// Maximum CPU utilization percent.
    pub max_cpu_percent: u64,
}

/// Health classification derived from node load and capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaNodeHealth {
    /// Node is operating normally.
    Healthy,
    /// Node is experiencing elevated load but can still accept work.
    Degraded,
    /// Node should not receive new work.
    Unhealthy,
}

/// Lifecycle status of a media node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeStatus {
    /// Node is active and eligible for new scheduling.
    Active,
    /// Node is draining; existing sessions continue but no new ones are scheduled.
    Draining,
    /// Node has left the cluster.
    Left,
}

/// Runtime view of a media node used for scheduling decisions.
#[derive(Clone, Debug)]
pub struct MediaNode {
    /// Node identity.
    pub node_id: NodeId,
    /// Identity of the current running instance.
    pub instance_id: String,
    /// Monotonic generation of the instance.
    pub instance_epoch: u64,
    /// Availability zone or region.
    pub zone: String,
    /// Region (may be a superset of the zone).
    pub region: String,
    /// Free-form node labels.
    pub labels: BTreeMap<String, String>,
    /// gRPC control endpoint address.
    pub control_endpoint: String,
    /// Media plane addresses advertised by the node.
    pub media_addresses: Vec<String>,
    /// Capabilities advertised by the node.
    pub capabilities: Vec<MediaCapability>,
    /// Capacity limits.
    pub capacity: MediaNodeCapacity,
    /// Current CPU/operation load percent.
    pub load: u64,
    /// Total session count used for scheduling (reported + reserved).
    pub session_count: u64,
    /// Derived health.
    pub health: MediaNodeHealth,
    /// Whether the node is draining.
    pub draining: bool,
    /// Lifecycle status.
    pub status: NodeStatus,
    /// Time of the last heartbeat.
    pub last_heartbeat_at: Option<UtcTimestamp>,
    /// Time at which the registration lease expires.
    pub lease_until: Option<UtcTimestamp>,
    /// Registration generation.
    pub generation: u64,
    /// Contract version supported by the node.
    pub contract_version: u32,
}

impl MediaNode {
    /// Returns the remaining sessions until capacity.
    pub fn available_sessions(&self) -> u64 {
        self.capacity
            .max_sessions
            .saturating_sub(self.session_count)
    }

    /// Returns the instance epoch as the domain newtype.
    pub fn instance_epoch_value(&self) -> MediaNodeInstanceEpoch {
        MediaNodeInstanceEpoch(self.instance_epoch)
    }

    /// True if the node can accept a new session for scheduling.
    pub fn has_capacity(&self) -> bool {
        self.available_sessions() > 0 && self.health != MediaNodeHealth::Unhealthy
    }
}

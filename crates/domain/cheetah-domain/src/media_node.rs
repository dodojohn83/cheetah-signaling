//! Media node model used by the scheduler and storage layers.

use crate::DomainError;
use cheetah_signal_types::{MediaNodeInstanceEpoch, NodeId, UtcTimestamp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Default maximum byte length of a generic media-node string field.
const DEFAULT_MAX_STRING_FIELD_BYTES: usize = 256;
/// Default maximum byte length of a control-plane or media-plane endpoint.
const DEFAULT_MAX_ENDPOINT_BYTES: usize = 2048;
/// Default maximum number of network zones advertised by a media node.
const DEFAULT_MAX_NETWORK_ZONES: usize = 64;
/// Default maximum number of capabilities advertised by a media node.
const DEFAULT_MAX_CAPABILITIES: usize = 64;
/// Default maximum number of operations advertised per capability.
const DEFAULT_MAX_CAPABILITY_OPERATIONS: usize = 64;
/// Default maximum number of constraint key-value pairs per capability.
const DEFAULT_MAX_CAPABILITY_CONSTRAINTS: usize = 64;
/// Default maximum number of media plane addresses advertised by a node.
const DEFAULT_MAX_MEDIA_ADDRESSES: usize = 256;
/// Default maximum number of node labels.
const DEFAULT_MAX_LABELS: usize = 256;
/// Default maximum byte length of a label value.
const DEFAULT_MAX_LABEL_VALUE_BYTES: usize = 1024;

/// Bounds used to validate `MediaNode` and `MediaCapability` contents.
///
/// The scheduler maps its own `MediaRegistryConfig` onto these limits so the
/// domain validation never rejects values accepted by the gRPC layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaNodeLimits {
    /// Maximum byte length of generic string fields (`instance_id`, `zone`,
    /// `region`, `protocol`, `runtime_state`, map keys, operation strings).
    pub max_string_field_bytes: usize,
    /// Maximum byte length of `control_endpoint` and `media_addresses` entries.
    pub max_endpoint_bytes: usize,
    /// Maximum number of `network_zones` entries.
    pub max_network_zones: usize,
    /// Maximum number of `capabilities` entries.
    pub max_capabilities: usize,
    /// Maximum number of `operations` per capability.
    pub max_capability_operations: usize,
    /// Maximum number of `constraints` key-value pairs per capability.
    pub max_capability_constraints: usize,
    /// Maximum number of `media_addresses` entries.
    pub max_media_addresses: usize,
    /// Maximum number of `labels` entries.
    pub max_labels: usize,
    /// Maximum byte length of a label value.
    pub max_label_value_bytes: usize,
}

impl Default for MediaNodeLimits {
    fn default() -> Self {
        Self::production()
    }
}

impl MediaNodeLimits {
    /// Returns a default production limit set.
    pub const fn production() -> Self {
        Self {
            max_string_field_bytes: DEFAULT_MAX_STRING_FIELD_BYTES,
            max_endpoint_bytes: DEFAULT_MAX_ENDPOINT_BYTES,
            max_network_zones: DEFAULT_MAX_NETWORK_ZONES,
            max_capabilities: DEFAULT_MAX_CAPABILITIES,
            max_capability_operations: DEFAULT_MAX_CAPABILITY_OPERATIONS,
            max_capability_constraints: DEFAULT_MAX_CAPABILITY_CONSTRAINTS,
            max_media_addresses: DEFAULT_MAX_MEDIA_ADDRESSES,
            max_labels: DEFAULT_MAX_LABELS,
            max_label_value_bytes: DEFAULT_MAX_LABEL_VALUE_BYTES,
        }
    }
}

/// A single capability advertised by a media node.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaCapability {
    /// Protocol this capability serves, such as `gb28181` or `onvif`.
    pub protocol: String,
    /// Operations supported under this capability, such as `live` or `playback`.
    pub operations: Vec<String>,
    /// Capability constraints, e.g. supported codecs or transports.
    pub constraints: BTreeMap<String, String>,
    /// Capability contract version.
    pub version: u64,
    /// Runtime state of this capability, e.g. `healthy` or `degraded`.
    pub runtime_state: String,
}

impl MediaCapability {
    /// Validates that all string/collection fields are within `limits`.
    pub fn validate(&self, limits: &MediaNodeLimits) -> Result<(), DomainError> {
        if self.protocol.len() > limits.max_string_field_bytes {
            return Err(DomainError::invalid_argument(format!(
                "capability protocol exceeds {} bytes",
                limits.max_string_field_bytes
            )));
        }
        if self.runtime_state.len() > limits.max_string_field_bytes {
            return Err(DomainError::invalid_argument(format!(
                "capability runtime_state exceeds {} bytes",
                limits.max_string_field_bytes
            )));
        }
        if self.operations.len() > limits.max_capability_operations {
            return Err(DomainError::invalid_argument(format!(
                "capability operations exceed {} entries",
                limits.max_capability_operations
            )));
        }
        for op in &self.operations {
            if op.len() > limits.max_string_field_bytes {
                return Err(DomainError::invalid_argument(
                    "capability operation string exceeds limit",
                ));
            }
        }
        if self.constraints.len() > limits.max_capability_constraints {
            return Err(DomainError::invalid_argument(format!(
                "capability constraints exceed {} entries",
                limits.max_capability_constraints
            )));
        }
        for (k, v) in &self.constraints {
            if k.len() > limits.max_string_field_bytes {
                return Err(DomainError::invalid_argument(
                    "capability constraint key exceeds limit",
                ));
            }
            if v.len() > limits.max_string_field_bytes {
                return Err(DomainError::invalid_argument(
                    "capability constraint value exceeds limit",
                ));
            }
        }
        Ok(())
    }
}

/// Capacity limits advertised by a media node.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaNodeCapacity {
    /// Maximum concurrent sessions.
    pub max_sessions: u64,
    /// Maximum bandwidth in megabits per second.
    pub max_bandwidth_mbps: u64,
    /// Maximum CPU utilization percent.
    pub max_cpu_percent: u64,
}

/// Health classification derived from node load and capacity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaNodeHealth {
    /// Node is operating normally.
    #[default]
    Healthy,
    /// Node is experiencing elevated load but can still accept work.
    Degraded,
    /// Node should not receive new work.
    Unhealthy,
}

/// Lifecycle status of a media node.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Node is active and eligible for new scheduling.
    #[default]
    Active,
    /// Node is draining; existing sessions continue but no new ones are scheduled.
    Draining,
    /// Node has left the cluster.
    Left,
}

/// Runtime view of a media node used for scheduling decisions.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
    /// Network zones the node can reach.
    pub network_zones: Vec<String>,
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
    /// Optimistic concurrency revision.
    pub revision: u64,
}

impl MediaNode {
    /// Returns the remaining sessions until capacity.
    pub fn available_sessions(&self) -> u64 {
        self.capacity
            .max_sessions
            .max(1)
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

    /// Validates that all string/collection fields are within `limits`.
    pub fn validate(&self, limits: &MediaNodeLimits) -> Result<(), DomainError> {
        for (name, value) in [
            ("instance_id", self.instance_id.as_str()),
            ("zone", self.zone.as_str()),
            ("region", self.region.as_str()),
        ] {
            if value.len() > limits.max_string_field_bytes {
                return Err(DomainError::invalid_argument(format!(
                    "{name} exceeds {} bytes",
                    limits.max_string_field_bytes
                )));
            }
        }
        if self.network_zones.len() > limits.max_network_zones {
            return Err(DomainError::invalid_argument(format!(
                "network_zones exceed {} entries",
                limits.max_network_zones
            )));
        }
        for zone in &self.network_zones {
            if zone.len() > limits.max_string_field_bytes {
                return Err(DomainError::invalid_argument(
                    "network_zone string exceeds limit",
                ));
            }
        }
        if self.labels.len() > limits.max_labels {
            return Err(DomainError::invalid_argument(format!(
                "labels exceed {} entries",
                limits.max_labels
            )));
        }
        for (k, v) in &self.labels {
            if k.len() > limits.max_string_field_bytes {
                return Err(DomainError::invalid_argument("label key exceeds limit"));
            }
            if v.len() > limits.max_label_value_bytes {
                return Err(DomainError::invalid_argument("label value exceeds limit"));
            }
        }
        if self.control_endpoint.len() > limits.max_endpoint_bytes {
            return Err(DomainError::invalid_argument(format!(
                "control_endpoint exceeds {} bytes",
                limits.max_endpoint_bytes
            )));
        }
        if self.media_addresses.len() > limits.max_media_addresses {
            return Err(DomainError::invalid_argument(format!(
                "media_addresses exceed {} entries",
                limits.max_media_addresses
            )));
        }
        for addr in &self.media_addresses {
            if addr.len() > limits.max_endpoint_bytes {
                return Err(DomainError::invalid_argument(
                    "media_address string exceeds limit",
                ));
            }
        }
        if self.capabilities.len() > limits.max_capabilities {
            return Err(DomainError::invalid_argument(format!(
                "capabilities exceed {} entries",
                limits.max_capabilities
            )));
        }
        for cap in &self.capabilities {
            cap.validate(limits)?;
        }
        Ok(())
    }

    /// Recomputes health from load, session count and capacity.
    pub fn recalc_health(&mut self) {
        let max_cpu = if self.capacity.max_cpu_percent == 0 {
            100
        } else {
            self.capacity.max_cpu_percent
        };
        let max_sessions = self.capacity.max_sessions.max(1);
        if self.load >= max_cpu || self.session_count >= max_sessions {
            self.health = MediaNodeHealth::Unhealthy;
        } else if self.load >= max_cpu.saturating_mul(7) / 10
            || self.session_count >= max_sessions.saturating_mul(7) / 10
        {
            self.health = MediaNodeHealth::Degraded;
        } else {
            self.health = MediaNodeHealth::Healthy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaCapability, MediaNode, MediaNodeLimits};
    use cheetah_signal_types::NodeId;

    #[test]
    fn media_capability_rejects_oversized_protocol() {
        let cap = MediaCapability {
            protocol: "x".repeat(MediaNodeLimits::production().max_string_field_bytes + 1),
            ..MediaCapability::default()
        };
        assert!(cap.validate(&MediaNodeLimits::production()).is_err());
    }

    #[test]
    fn media_node_rejects_oversized_control_endpoint() {
        let node = MediaNode {
            node_id: NodeId::from(uuid::Uuid::nil()),
            control_endpoint: "http://".to_string()
                + &"x".repeat(MediaNodeLimits::production().max_endpoint_bytes + 1),
            ..MediaNode::default()
        };
        assert!(node.validate(&MediaNodeLimits::production()).is_err());
    }

    #[test]
    fn media_node_rejects_too_many_capabilities() {
        let node = MediaNode {
            node_id: NodeId::from(uuid::Uuid::nil()),
            capabilities: (0..MediaNodeLimits::production().max_capabilities + 1)
                .map(|_| MediaCapability::default())
                .collect(),
            ..MediaNode::default()
        };
        assert!(node.validate(&MediaNodeLimits::production()).is_err());
    }

    #[test]
    fn media_node_accepts_default() {
        let node = MediaNode {
            node_id: NodeId::from(uuid::Uuid::nil()),
            ..MediaNode::default()
        };
        assert!(node.validate(&MediaNodeLimits::production()).is_ok());
    }
}

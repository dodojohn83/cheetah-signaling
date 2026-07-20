//! In-memory media node registry.

use crate::config::MediaRegistryConfig;
use crate::error::SchedulerError;
use crate::model::{MediaNode, MediaNodeHealth, NodeStatus};
use cheetah_signal_types::{Clock, MediaBindingId, NodeId, TenantId, UtcTimestamp};
use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Debug)]
pub(crate) struct NodeEntry {
    pub(crate) node: MediaNode,
    pub(crate) reported_session_count: u64,
    /// Active reservations keyed by tenant and binding, with expiry time.
    pub(crate) reserved: BTreeMap<(TenantId, MediaBindingId), UtcTimestamp>,
    pub(crate) instance_id: String,
}

/// Registry of media nodes used for scheduling and gRPC lifecycle.
#[async_trait::async_trait]
pub trait MediaNodeRegistry: Send + Sync {
    /// Registers or re-registers a media node.
    async fn register(
        &self,
        node: MediaNode,
        lease_ttl_ms: u64,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError>;

    /// Records a heartbeat from a media node.
    async fn heartbeat(
        &self,
        node_id: NodeId,
        load: u64,
        session_count: u64,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError>;

    /// Marks a node as draining or active.
    async fn drain(
        &self,
        node_id: NodeId,
        drain: bool,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError>;

    /// Marks a node as left.
    async fn deregister(
        &self,
        node_id: NodeId,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError>;

    /// Returns the current runtime view of a node, if known.
    async fn get(&self, node_id: NodeId, clock: &dyn Clock) -> Option<MediaNode>;

    /// Lists nodes that are not left and whose lease has not expired.
    async fn list_active(&self, clock: &dyn Clock) -> Vec<MediaNode>;

    /// Reserves capacity for a media binding on the given node.
    async fn reserve(
        &self,
        node_id: NodeId,
        tenant_id: TenantId,
        binding_id: MediaBindingId,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError>;

    /// Releases a previously reserved binding.
    async fn release(
        &self,
        node_id: NodeId,
        tenant_id: TenantId,
        binding_id: MediaBindingId,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError>;
}

/// In-memory implementation of the media node registry.
pub struct InMemoryMediaNodeRegistry {
    config: MediaRegistryConfig,
    nodes: RwLock<BTreeMap<NodeId, NodeEntry>>,
}

impl InMemoryMediaNodeRegistry {
    /// Creates a new registry with the provided configuration.
    pub fn new(config: MediaRegistryConfig) -> Self {
        Self {
            config,
            nodes: RwLock::new(BTreeMap::new()),
        }
    }
}

impl Default for InMemoryMediaNodeRegistry {
    fn default() -> Self {
        Self::new(MediaRegistryConfig::default())
    }
}

impl std::fmt::Debug for InMemoryMediaNodeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryMediaNodeRegistry")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl MediaNodeRegistry for InMemoryMediaNodeRegistry {
    async fn register(
        &self,
        node: MediaNode,
        lease_ttl_ms: u64,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| SchedulerError::InvalidArgument("registry lock poisoned".to_string()))?;
        let now = clock.now_wall();
        let lease = lease_until(clock, lease_ttl_ms);
        let instance_id = node.instance_id.clone();
        let entry = if let Some(existing) = nodes.get(&node.node_id) {
            if existing.instance_id == node.instance_id {
                let mut updated = node;
                updated.generation = existing.node.generation;
                updated.instance_epoch = existing.node.instance_epoch;
                updated.status = NodeStatus::Active;
                updated.draining = false;
                updated.load = existing.node.load;
                updated.last_heartbeat_at = Some(now);
                updated.lease_until = lease;
                NodeEntry {
                    node: updated,
                    reported_session_count: existing.reported_session_count,
                    reserved: existing.reserved.clone(),
                    instance_id,
                }
            } else {
                let mut updated = node;
                updated.generation = existing.node.generation.saturating_add(1);
                updated.instance_epoch = existing.node.instance_epoch.saturating_add(1);
                updated.status = NodeStatus::Active;
                updated.draining = false;
                updated.last_heartbeat_at = Some(now);
                updated.lease_until = lease;
                NodeEntry {
                    node: updated,
                    reported_session_count: 0,
                    reserved: BTreeMap::new(),
                    instance_id,
                }
            }
        } else {
            let mut updated = node;
            updated.generation = 1;
            updated.instance_epoch = 1;
            updated.status = NodeStatus::Active;
            updated.draining = false;
            updated.last_heartbeat_at = Some(now);
            updated.lease_until = lease;
            NodeEntry {
                instance_id,
                node: updated,
                reported_session_count: 0,
                reserved: BTreeMap::new(),
            }
        };
        let view = to_media_node(&entry, now, &self.config);
        nodes.insert(view.node_id, entry);
        Ok(view)
    }

    async fn heartbeat(
        &self,
        node_id: NodeId,
        load: u64,
        session_count: u64,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        if load > self.config.max_reported_load_percent {
            return Err(SchedulerError::InvalidArgument(format!(
                "load exceeds {}%",
                self.config.max_reported_load_percent
            )));
        }
        if session_count > self.config.max_reported_session_count {
            return Err(SchedulerError::InvalidArgument(format!(
                "session_count exceeds {}",
                self.config.max_reported_session_count
            )));
        }

        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| SchedulerError::InvalidArgument("registry lock poisoned".to_string()))?;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        if entry.node.status == NodeStatus::Left {
            return Err(SchedulerError::NodeNotFound(format!(
                "{node_id} has been deregistered"
            )));
        }
        let now = clock.now_wall();
        entry.node.load = load;
        entry.reported_session_count = session_count;
        entry.node.lease_until = lease_until(clock, self.config.default_lease_ttl_ms);
        entry.node.last_heartbeat_at = Some(now);
        Ok(to_media_node(entry, now, &self.config))
    }

    async fn drain(
        &self,
        node_id: NodeId,
        drain: bool,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| SchedulerError::InvalidArgument("registry lock poisoned".to_string()))?;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        entry.node.status = if drain {
            NodeStatus::Draining
        } else {
            NodeStatus::Active
        };
        entry.node.draining = drain;
        let now = clock.now_wall();
        Ok(to_media_node(entry, now, &self.config))
    }

    async fn deregister(
        &self,
        node_id: NodeId,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| SchedulerError::InvalidArgument("registry lock poisoned".to_string()))?;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        entry.node.status = NodeStatus::Left;
        entry.node.lease_until = None;
        let now = clock.now_wall();
        Ok(to_media_node(entry, now, &self.config))
    }

    async fn get(&self, node_id: NodeId, clock: &dyn Clock) -> Option<MediaNode> {
        let nodes = self.nodes.read().ok()?;
        let now = clock.now_wall();
        nodes
            .get(&node_id)
            .map(|e| to_media_node(e, now, &self.config))
    }

    async fn list_active(&self, clock: &dyn Clock) -> Vec<MediaNode> {
        let nodes = match self.nodes.read() {
            Ok(n) => n,
            Err(_) => return Vec::new(),
        };
        let now = clock.now_wall();
        nodes
            .values()
            .filter(|e| is_active(e, now, &self.config))
            .map(|e| to_media_node(e, now, &self.config))
            .collect()
    }

    async fn reserve(
        &self,
        node_id: NodeId,
        tenant_id: TenantId,
        binding_id: MediaBindingId,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| SchedulerError::InvalidArgument("registry lock poisoned".to_string()))?;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        let now = clock.now_wall();
        let ttl = i64::try_from(self.config.reservation_ttl_ms).unwrap_or(i64::MAX);
        let deadline = now
            .checked_add(cheetah_signal_types::DurationMs::from_millis(ttl))
            .ok_or_else(|| {
                SchedulerError::InvalidArgument("reservation deadline overflow".to_string())
            })?;
        entry.reserved.retain(|_, d| *d > now);
        let active = entry.reserved.len() as u64;
        let total = entry.reported_session_count.saturating_add(active);
        if total >= entry.node.capacity.max_sessions.max(1) {
            return Err(SchedulerError::CapacityExhausted(node_id.to_string()));
        }
        entry.reserved.insert((tenant_id, binding_id), deadline);
        Ok(to_media_node(entry, now, &self.config))
    }

    async fn release(
        &self,
        node_id: NodeId,
        tenant_id: TenantId,
        binding_id: MediaBindingId,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut nodes = self
            .nodes
            .write()
            .map_err(|_| SchedulerError::InvalidArgument("registry lock poisoned".to_string()))?;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        let now = clock.now_wall();
        entry.reserved.remove(&(tenant_id, binding_id));
        entry.reserved.retain(|_, d| *d > now);
        Ok(to_media_node(entry, now, &self.config))
    }
}

pub(crate) fn to_media_node(
    entry: &NodeEntry,
    now: UtcTimestamp,
    config: &MediaRegistryConfig,
) -> MediaNode {
    let active_reserved = entry
        .reserved
        .values()
        .filter(|deadline| **deadline > now)
        .count() as u64;
    let total_sessions = entry.reported_session_count.saturating_add(active_reserved);
    let mut node = entry.node.clone();
    node.session_count = total_sessions;
    node.recalc_health();
    if is_stale(entry, now, config) {
        node.health = MediaNodeHealth::Unhealthy;
    }
    node
}

pub(crate) fn is_active(
    entry: &NodeEntry,
    now: UtcTimestamp,
    config: &MediaRegistryConfig,
) -> bool {
    entry.node.status != NodeStatus::Left
        && !is_lease_expired(entry, now)
        && !is_stale(entry, now, config)
}

pub(crate) fn is_lease_expired(entry: &NodeEntry, now: UtcTimestamp) -> bool {
    match entry.node.lease_until {
        None => false,
        Some(lease) => now >= lease,
    }
}

pub(crate) fn is_stale(entry: &NodeEntry, now: UtcTimestamp, config: &MediaRegistryConfig) -> bool {
    match entry.node.last_heartbeat_at {
        None => true,
        Some(last) => {
            let elapsed = now.as_offset() - last.as_offset();
            elapsed
                >= time::Duration::milliseconds(
                    i64::try_from(config.heartbeat_timeout_ms).unwrap_or(i64::MAX),
                )
        }
    }
}

pub(crate) fn lease_until(clock: &dyn Clock, ttl_ms: u64) -> Option<UtcTimestamp> {
    let now = clock.now_wall();
    let ttl = i64::try_from(ttl_ms).unwrap_or(i64::MAX);
    now.checked_add(cheetah_signal_types::DurationMs::from_millis(ttl))
}

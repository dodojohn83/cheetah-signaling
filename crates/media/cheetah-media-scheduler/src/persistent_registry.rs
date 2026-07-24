//! Persistent media node registry backed by a `MediaNodeRepository`.

use crate::config::MediaRegistryConfig;
use crate::error::SchedulerError;
use crate::model::{MediaNode, NodeStatus};
use crate::registry::{MediaNodeRegistry, NodeEntry, is_active, lease_until, to_media_node};
use cheetah_domain::DomainEvent;
use cheetah_signal_types::{
    Clock, Event, IdGenerator, MAX_PAGE_SIZE, MediaBindingId, NodeId, PageRequest, Principal,
    PrincipalKind, RequestContext, ResourceId, ResourceKind, ResourceRef, TenantId,
};
use cheetah_storage_api::MediaNodeRepository;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// A `MediaNodeRegistry` that persists node metadata through a
/// `MediaNodeRepository` while keeping the in-memory reservation map.
pub struct PersistentMediaNodeRegistry {
    config: MediaRegistryConfig,
    repo: Arc<Mutex<Box<dyn MediaNodeRepository>>>,
    nodes: RwLock<BTreeMap<NodeId, NodeEntry>>,
    id_generator: Arc<dyn IdGenerator>,
    node_id: NodeId,
}

impl std::fmt::Debug for PersistentMediaNodeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistentMediaNodeRegistry")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl PersistentMediaNodeRegistry {
    /// Creates a new registry backed by `repo`.
    pub fn new(
        config: MediaRegistryConfig,
        repo: Box<dyn MediaNodeRepository>,
        id_generator: Arc<dyn IdGenerator>,
        node_id: NodeId,
    ) -> Self {
        Self {
            config,
            repo: Arc::new(Mutex::new(repo)),
            nodes: RwLock::new(BTreeMap::new()),
            id_generator,
            node_id,
        }
    }

    /// Builds a `MediaNodeUpdated` outbox event for the given node.
    ///
    /// The payload's `node` fields are provisional; the storage layer overwrites
    /// them with the persisted row inside the same transaction, so callers only
    /// need to supply a node with the correct `node_id`.
    fn make_event(&self, clock: &dyn Clock, node: &MediaNode) -> Event<DomainEvent> {
        let message_id = self.id_generator.generate_message_id();
        let correlation_id = self.id_generator.generate_correlation_id();
        let ctx = RequestContext {
            tenant_id: TenantId::default(),
            principal: Principal {
                id: self.node_id.to_string(),
                kind: PrincipalKind::Service,
                scopes: Vec::new(),
            },
            message_id,
            correlation_id,
            traceparent: None,
            tracestate: None,
            deadline: None,
            node_id: Some(self.node_id),
            source_ip: None,
        };
        let aggregate_ref = ResourceRef {
            tenant_id: TenantId::default(),
            kind: ResourceKind::MediaNode,
            id: ResourceId::MediaNode(node.node_id),
        };
        Event::new(
            self.id_generator.as_ref(),
            clock,
            &ctx,
            TenantId::default(),
            aggregate_ref,
            node.revision,
            DomainEvent::MediaNodeUpdated { node: node.clone() },
        )
    }

    /// Loads currently alive nodes from the repository into memory.
    pub async fn load(&self, clock: &dyn Clock) -> Result<(), SchedulerError> {
        let mut nodes = self.nodes.write().await;
        let mut cursor = None;
        loop {
            let page_request = PageRequest {
                cursor,
                page_size: MAX_PAGE_SIZE,
            };
            let page = self
                .repo
                .lock()
                .await
                .list_alive(clock.now_wall(), page_request)
                .await
                .map_err(|e| SchedulerError::Backend(e.to_string()))?;

            let has_more = page.next_cursor.is_some();
            for node in page.items {
                let entry = NodeEntry {
                    node: node.clone(),
                    reported_session_count: node.session_count,
                    reserved: BTreeMap::new(),
                    instance_id: node.instance_id.clone(),
                };
                nodes.insert(node.node_id, entry);
            }
            cursor = page.next_cursor;
            if !has_more {
                break;
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl MediaNodeRegistry for PersistentMediaNodeRegistry {
    async fn register(
        &self,
        node: MediaNode,
        lease_ttl_ms: u64,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut nodes = self.nodes.write().await;
        let now = clock.now_wall();
        let lease = lease_until(clock, lease_ttl_ms);

        let (updated, reported, reserved) = if let Some(existing) = nodes.get(&node.node_id) {
            if existing.instance_id == node.instance_id {
                let mut updated = node;
                updated.generation = existing.node.generation;
                updated.instance_epoch = existing.node.instance_epoch;
                updated.status = NodeStatus::Active;
                updated.draining = false;
                updated.load = existing.node.load;
                updated.last_heartbeat_at = Some(now);
                updated.lease_until = lease;
                updated.revision = existing.node.revision;
                (
                    updated,
                    existing.reported_session_count,
                    existing.reserved.clone(),
                )
            } else {
                let mut updated = node;
                updated.generation = existing.node.generation.saturating_add(1);
                updated.instance_epoch = existing.node.instance_epoch.saturating_add(1);
                updated.status = NodeStatus::Active;
                updated.draining = false;
                updated.last_heartbeat_at = Some(now);
                updated.lease_until = lease;
                updated.revision = existing.node.revision;
                (updated, 0, BTreeMap::new())
            }
        } else {
            let mut updated = node;
            updated.generation = 1;
            updated.instance_epoch = 1;
            updated.status = NodeStatus::Active;
            updated.draining = false;
            updated.last_heartbeat_at = Some(now);
            updated.lease_until = lease;
            updated.revision = 0;
            (updated, 0, BTreeMap::new())
        };

        let event = self.make_event(clock, &updated);
        let persisted = self
            .repo
            .lock()
            .await
            .register(updated, vec![event])
            .await
            .map_err(|e| SchedulerError::Backend(e.to_string()))?;

        let entry = NodeEntry {
            node: persisted.clone(),
            reported_session_count: reported,
            reserved,
            instance_id: persisted.instance_id.clone(),
        };
        let view = to_media_node(&entry, now, &self.config);
        nodes.insert(view.node_id, entry);
        Ok(view)
    }

    async fn heartbeat(
        &self,
        node_id: NodeId,
        lease_id: &str,
        instance_epoch: u64,
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

        let mut nodes = self.nodes.write().await;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        if entry.node.status == NodeStatus::Left {
            return Err(SchedulerError::NodeNotFound(format!(
                "{node_id} has been deregistered"
            )));
        }
        if entry.instance_id != lease_id || entry.node.instance_epoch != instance_epoch {
            return Err(SchedulerError::NodeNotFound(format!(
                "{node_id} lease or instance epoch mismatch"
            )));
        }
        let now = clock.now_wall();
        let lease = lease_until(clock, self.config.default_lease_ttl_ms)
            .ok_or_else(|| SchedulerError::Backend("lease timestamp overflow".to_string()))?;

        let event = self.make_event(clock, &entry.node);

        let persisted = self
            .repo
            .lock()
            .await
            .heartbeat(
                node_id,
                lease_id.to_string(),
                lease,
                now,
                load,
                session_count,
                vec![event],
            )
            .await
            .map_err(|e| SchedulerError::Backend(e.to_string()))?;

        match persisted {
            Some(node) => {
                entry.node = node;
                entry.reported_session_count = session_count;
                entry.node.last_heartbeat_at = Some(now);
                entry.node.lease_until = Some(lease);
                Ok(to_media_node(entry, now, &self.config))
            }
            None => Err(SchedulerError::NodeNotFound(node_id.to_string())),
        }
    }

    async fn drain(
        &self,
        node_id: NodeId,
        drain: bool,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut nodes = self.nodes.write().await;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        let now = clock.now_wall();
        let instance_id = entry.instance_id.clone();

        let event = self.make_event(clock, &entry.node);

        let persisted = self
            .repo
            .lock()
            .await
            .set_draining(node_id, instance_id, drain, now, vec![event])
            .await
            .map_err(|e| SchedulerError::Backend(e.to_string()))?;

        match persisted {
            Some(node) => {
                entry.node = node;
                Ok(to_media_node(entry, now, &self.config))
            }
            None => Err(SchedulerError::NodeNotFound(node_id.to_string())),
        }
    }

    async fn deregister(
        &self,
        node_id: NodeId,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut nodes = self.nodes.write().await;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        let now = clock.now_wall();
        let instance_id = entry.instance_id.clone();
        let protection_lease = lease_until(clock, self.config.deregister_protection_ttl_ms);

        let event = self.make_event(clock, &entry.node);

        let persisted = self
            .repo
            .lock()
            .await
            .deregister(node_id, instance_id, now, protection_lease, vec![event])
            .await
            .map_err(|e| SchedulerError::Backend(e.to_string()))?;

        match persisted {
            Some(node) => {
                entry.node = node;
                entry.node.lease_until = protection_lease;
                Ok(to_media_node(entry, now, &self.config))
            }
            None => Err(SchedulerError::NodeNotFound(node_id.to_string())),
        }
    }

    async fn get(&self, node_id: NodeId, clock: &dyn Clock) -> Option<MediaNode> {
        let now = clock.now_wall();
        {
            let nodes = self.nodes.read().await;
            if let Some(entry) = nodes.get(&node_id) {
                return Some(to_media_node(entry, now, &self.config));
            }
        }

        let fetched = self.repo.lock().await.get(node_id).await;
        let node = fetched.ok().flatten()?;
        let entry = NodeEntry {
            node: node.clone(),
            reported_session_count: node.session_count,
            reserved: BTreeMap::new(),
            instance_id: node.instance_id.clone(),
        };
        if !is_active(&entry, now, &self.config) {
            return None;
        }
        let view = to_media_node(&entry, now, &self.config);
        {
            let mut nodes = self.nodes.write().await;
            nodes.entry(node_id).or_insert(entry);
        }
        Some(view)
    }

    async fn list_active(&self, clock: &dyn Clock) -> Vec<MediaNode> {
        let now = clock.now_wall();
        {
            let mut nodes = self.nodes.write().await;
            if nodes.is_empty() {
                let mut cursor = None;
                loop {
                    let page_request = PageRequest {
                        cursor,
                        page_size: MAX_PAGE_SIZE,
                    };
                    if let Ok(page) = self.repo.lock().await.list_alive(now, page_request).await {
                        let has_more = page.next_cursor.is_some();
                        for node in page.items {
                            let entry = NodeEntry {
                                node: node.clone(),
                                reported_session_count: node.session_count,
                                reserved: BTreeMap::new(),
                                instance_id: node.instance_id.clone(),
                            };
                            nodes.insert(node.node_id, entry);
                        }
                        cursor = page.next_cursor;
                        if !has_more {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
        }

        let nodes = self.nodes.read().await;
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
        let mut nodes = self.nodes.write().await;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        if entry.node.draining || entry.node.status == NodeStatus::Draining {
            return Err(SchedulerError::NodeDraining(node_id.to_string()));
        }
        if entry.node.status == NodeStatus::Left {
            return Err(SchedulerError::NodeNotFound(node_id.to_string()));
        }
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
        let mut nodes = self.nodes.write().await;
        let entry = nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
        let now = clock.now_wall();
        entry.reserved.remove(&(tenant_id, binding_id));
        entry.reserved.retain(|_, d| *d > now);
        Ok(to_media_node(entry, now, &self.config))
    }
}

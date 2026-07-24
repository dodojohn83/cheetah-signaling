//! Persistent media node registry backed by a `MediaNodeRepository`.

use crate::config::MediaRegistryConfig;
use crate::error::SchedulerError;
use crate::model::{MediaNode, MediaNodeHealth, NodeStatus};
use crate::registry::{
    MediaNodeRegistry, NodeEntry, is_active, is_lease_expired, lease_until, to_media_node,
};
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
        let mut collected = BTreeMap::new();
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
                .map_err(SchedulerError::backend)?;

            let has_more = page.next_cursor.is_some();
            for node in page.items {
                let entry = NodeEntry {
                    node: node.clone(),
                    reported_session_count: node.session_count,
                    reserved: BTreeMap::new(),
                    instance_id: node.instance_id.clone(),
                };
                collected.insert(node.node_id, entry);
            }
            cursor = page.next_cursor;
            if !has_more {
                break;
            }
        }

        let mut nodes = self.nodes.write().await;
        nodes.extend(collected);
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
        let now = clock.now_wall();
        let lease = lease_until(clock, lease_ttl_ms);
        let instance_id = node.instance_id.clone();

        // Snapshot the in-memory entry (if any) without holding the lock across
        // repository I/O.
        let in_memory = {
            let nodes = self.nodes.read().await;
            nodes.get(&node.node_id).map(|entry| {
                (
                    entry.node.clone(),
                    entry.instance_id.clone(),
                    entry.reported_session_count,
                    entry.reserved.clone(),
                )
            })
        };

        let (updated, reported, reserved) =
            if let Some((existing_node, existing_instance_id, reported, reserved)) = in_memory {
                if existing_instance_id == instance_id {
                    let mut updated = node;
                    updated.generation = existing_node.generation;
                    updated.instance_epoch = existing_node.instance_epoch;
                    updated.status = NodeStatus::Active;
                    updated.draining = false;
                    updated.load = existing_node.load;
                    updated.last_heartbeat_at = Some(now);
                    updated.lease_until = lease;
                    updated.revision = existing_node.revision;
                    (updated, reported, reserved)
                } else {
                    let mut updated = node;
                    updated.generation = existing_node.generation.saturating_add(1);
                    updated.instance_epoch = existing_node.instance_epoch.saturating_add(1);
                    updated.status = NodeStatus::Active;
                    updated.draining = false;
                    updated.last_heartbeat_at = Some(now);
                    updated.lease_until = lease;
                    updated.revision = existing_node.revision;
                    (updated, 0, BTreeMap::new())
                }
            } else {
                let existing = self
                    .repo
                    .lock()
                    .await
                    .get(node.node_id)
                    .await
                    .map_err(|e| SchedulerError::Backend(e.to_string()))?;
                if let Some(existing_node) = existing {
                    let mut updated = node;
                    updated.generation = existing_node.generation.saturating_add(1);
                    updated.instance_epoch = existing_node.instance_epoch.saturating_add(1);
                    updated.status = NodeStatus::Active;
                    updated.draining = false;
                    updated.last_heartbeat_at = Some(now);
                    updated.lease_until = lease;
                    updated.revision = existing_node.revision;
                    (updated, 0, BTreeMap::new())
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
                }
            };

        let event = self.make_event(clock, &updated);
        let persisted = self
            .repo
            .lock()
            .await
            .register(updated, vec![event])
            .await
            .map_err(SchedulerError::backend)?;

        let entry = NodeEntry {
            node: persisted.clone(),
            reported_session_count: reported,
            reserved,
            instance_id: persisted.instance_id.clone(),
        };
        let view = to_media_node(&entry, now, &self.config);
        {
            let mut nodes = self.nodes.write().await;
            nodes.insert(view.node_id, entry);
        }
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

        let now = clock.now_wall();
        let lease = lease_until(clock, self.config.default_lease_ttl_ms)
            .ok_or_else(|| SchedulerError::backend("lease timestamp overflow"))?;

        // Validate the in-memory entry and build the outbox event without
        // holding the write lock across the repository I/O.
        let event = {
            let nodes = self.nodes.read().await;
            let entry = nodes
                .get(&node_id)
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
            self.make_event(clock, &entry.node)
        };

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
            .map_err(SchedulerError::backend)?;

        let mut nodes = self.nodes.write().await;
        match persisted {
            Some(node) => {
                let entry = nodes
                    .get_mut(&node_id)
                    .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
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
        let now = clock.now_wall();

        // Snapshot the instance id and build the outbox event without holding
        // the write lock across the repository I/O.
        let (instance_id, event) = {
            let nodes = self.nodes.read().await;
            let entry = nodes
                .get(&node_id)
                .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
            (
                entry.instance_id.clone(),
                self.make_event(clock, &entry.node),
            )
        };

        let persisted = self
            .repo
            .lock()
            .await
            .set_draining(node_id, instance_id, drain, now, vec![event])
            .await
            .map_err(SchedulerError::backend)?;

        let mut nodes = self.nodes.write().await;
        match persisted {
            Some(node) => {
                let entry = nodes
                    .get_mut(&node_id)
                    .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
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
        let now = clock.now_wall();
        let protection_lease = lease_until(clock, self.config.deregister_protection_ttl_ms);

        // Snapshot the instance id and build the outbox event without holding
        // the write lock across the repository I/O.
        let (instance_id, event) = {
            let nodes = self.nodes.read().await;
            let entry = nodes
                .get(&node_id)
                .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
            (
                entry.instance_id.clone(),
                self.make_event(clock, &entry.node),
            )
        };

        let persisted = self
            .repo
            .lock()
            .await
            .deregister(node_id, instance_id, now, protection_lease, vec![event])
            .await
            .map_err(SchedulerError::backend)?;

        let mut nodes = self.nodes.write().await;
        match persisted {
            Some(node) => {
                let entry = nodes
                    .get_mut(&node_id)
                    .ok_or_else(|| SchedulerError::NodeNotFound(node_id.to_string()))?;
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
            if let Some(entry) = nodes.get(&node_id)
                && is_active(entry, now, &self.config)
            {
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
        let should_load = {
            let nodes = self.nodes.read().await;
            nodes.is_empty()
        };

        if should_load {
            let mut collected = BTreeMap::new();
            let mut cursor = None;
            let mut load_ok = true;
            loop {
                let page_request = PageRequest {
                    cursor,
                    page_size: MAX_PAGE_SIZE,
                };
                match self.repo.lock().await.list_alive(now, page_request).await {
                    Ok(page) => {
                        let has_more = page.next_cursor.is_some();
                        for node in page.items {
                            let entry = NodeEntry {
                                node: node.clone(),
                                reported_session_count: node.session_count,
                                reserved: BTreeMap::new(),
                                instance_id: node.instance_id.clone(),
                            };
                            collected.insert(node.node_id, entry);
                        }
                        cursor = page.next_cursor;
                        if !has_more {
                            break;
                        }
                    }
                    Err(_) => {
                        load_ok = false;
                        break;
                    }
                }
            }

            if load_ok {
                let mut nodes = self.nodes.write().await;
                if nodes.is_empty() {
                    nodes.extend(collected);
                }
            } else {
                tracing::warn!(
                    "failed to load active media nodes from repository; not caching partial results"
                );
            }
        }

        let mut nodes = self.nodes.write().await;
        nodes.retain(|_, entry| {
            !(entry.node.status == NodeStatus::Left && is_lease_expired(entry, now))
        });
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
        let now = clock.now_wall();
        let view = to_media_node(entry, now, &self.config);
        if view.status == NodeStatus::Left {
            return Err(SchedulerError::NodeNotFound(node_id.to_string()));
        }
        if !is_active(entry, now, &self.config) {
            return Err(SchedulerError::NodeNotFound(node_id.to_string()));
        }
        if view.status == NodeStatus::Draining || view.draining {
            return Err(SchedulerError::NodeDraining(node_id.to_string()));
        }
        if view.health == MediaNodeHealth::Unhealthy {
            return Err(SchedulerError::CapacityExhausted(node_id.to_string()));
        }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::MediaRegistryConfig;
    use crate::model::{MediaNode, MediaNodeHealth, NodeStatus};
    use cheetah_signal_types::test_support::{FakeClock, FakeIdGenerator};
    use cheetah_signal_types::{DurationMs, NodeId, Page, PageRequest, UtcTimestamp};
    use cheetah_storage_api::{MediaNodeRepository, StorageError};
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeRepo {
        node: Arc<Mutex<Option<MediaNode>>>,
    }

    #[async_trait::async_trait]
    impl MediaNodeRepository for FakeRepo {
        async fn register(
            &mut self,
            mut node: MediaNode,
            _events: Vec<Event<DomainEvent>>,
        ) -> Result<MediaNode, StorageError> {
            node.revision = node.revision.saturating_add(1);
            *self.node.lock().unwrap() = Some(node.clone());
            Ok(node)
        }

        async fn heartbeat(
            &mut self,
            _node_id: NodeId,
            instance_id: String,
            lease_until: UtcTimestamp,
            updated_at: UtcTimestamp,
            load: u64,
            session_count: u64,
            _events: Vec<Event<DomainEvent>>,
        ) -> Result<Option<MediaNode>, StorageError> {
            let mut guard = self.node.lock().unwrap();
            if let Some(node) = guard.as_mut() {
                if node.instance_id != instance_id || node.status == NodeStatus::Left {
                    return Ok(None);
                }
                node.load = load;
                node.session_count = session_count;
                node.last_heartbeat_at = Some(updated_at);
                node.lease_until = Some(lease_until);
                node.revision = node.revision.saturating_add(1);
                Ok(Some(node.clone()))
            } else {
                Ok(None)
            }
        }

        async fn get(&self, _node_id: NodeId) -> Result<Option<MediaNode>, StorageError> {
            Ok(self.node.lock().unwrap().clone())
        }

        async fn list_alive(
            &self,
            now: UtcTimestamp,
            _page: PageRequest,
        ) -> Result<Page<MediaNode>, StorageError> {
            let node = self.node.lock().unwrap().clone();
            let items = if let Some(node) = node {
                if node.lease_until.is_some_and(|lease| now < lease) {
                    vec![node]
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            Ok(Page::new(items))
        }

        async fn set_draining(
            &mut self,
            _node_id: NodeId,
            instance_id: String,
            draining: bool,
            _updated_at: UtcTimestamp,
            _events: Vec<Event<DomainEvent>>,
        ) -> Result<Option<MediaNode>, StorageError> {
            let mut guard = self.node.lock().unwrap();
            if let Some(node) = guard.as_mut() {
                if node.instance_id != instance_id {
                    return Ok(None);
                }
                node.draining = draining;
                node.status = if draining {
                    NodeStatus::Draining
                } else {
                    NodeStatus::Active
                };
                node.revision = node.revision.saturating_add(1);
                Ok(Some(node.clone()))
            } else {
                Ok(None)
            }
        }

        async fn deregister(
            &mut self,
            _node_id: NodeId,
            instance_id: String,
            _updated_at: UtcTimestamp,
            lease_until: Option<UtcTimestamp>,
            _events: Vec<Event<DomainEvent>>,
        ) -> Result<Option<MediaNode>, StorageError> {
            let mut guard = self.node.lock().unwrap();
            if let Some(node) = guard.as_mut() {
                if node.instance_id != instance_id {
                    return Ok(None);
                }
                node.status = NodeStatus::Left;
                node.lease_until = lease_until;
                node.revision = node.revision.saturating_add(1);
                Ok(Some(node.clone()))
            } else {
                Ok(None)
            }
        }
    }

    fn test_node(node_id: NodeId, instance_id: &str) -> MediaNode {
        MediaNode {
            node_id,
            instance_id: instance_id.to_string(),
            instance_epoch: 0,
            generation: 0,
            revision: 0,
            zone: "zone-a".to_string(),
            region: "region-1".to_string(),
            network_zones: Vec::new(),
            labels: BTreeMap::new(),
            control_endpoint: "http://127.0.0.1:9000".to_string(),
            media_addresses: Vec::new(),
            capabilities: Vec::new(),
            capacity: crate::model::MediaNodeCapacity {
                max_sessions: 10,
                max_bandwidth_mbps: 100,
                max_cpu_percent: 100,
            },
            load: 0,
            session_count: 0,
            health: MediaNodeHealth::Healthy,
            draining: false,
            status: NodeStatus::Active,
            last_heartbeat_at: None,
            lease_until: None,
            contract_version: 0,
        }
    }

    fn test_config() -> MediaRegistryConfig {
        MediaRegistryConfig::test()
    }

    #[tokio::test]
    async fn register_fetches_persisted_epoch_when_in_memory_entry_missing() {
        let clock = Arc::new(FakeClock::new());
        let id_gen: Arc<dyn IdGenerator> = Arc::new(FakeIdGenerator::default());
        let repo = FakeRepo {
            node: Arc::new(Mutex::new(None)),
        };

        let node_id = NodeId::from_str("11111111-1111-1111-1111-111111111111").unwrap();

        let registry1 = PersistentMediaNodeRegistry::new(
            test_config(),
            Box::new(repo.clone()),
            id_gen.clone(),
            node_id,
        );

        let node1 = test_node(node_id, "instance-1");
        let registered1 = registry1
            .register(node1, 1000, clock.as_ref())
            .await
            .unwrap();
        assert_eq!(registered1.instance_epoch, 1);
        assert_eq!(registered1.generation, 1);

        registry1.deregister(node_id, clock.as_ref()).await.unwrap();

        let node2 = test_node(node_id, "instance-2");
        let registry2 = PersistentMediaNodeRegistry::new(
            test_config(),
            Box::new(repo.clone()),
            id_gen.clone(),
            node_id,
        );
        let registered2 = registry2
            .register(node2, 1000, clock.as_ref())
            .await
            .unwrap();
        assert_eq!(registered2.instance_epoch, 2);
        assert_eq!(registered2.generation, 2);
        assert_eq!(registered2.status, NodeStatus::Active);
    }

    #[tokio::test]
    async fn list_active_evicts_left_nodes_after_protection_lease_expires() {
        let clock = Arc::new(FakeClock::new());
        let id_gen: Arc<dyn IdGenerator> = Arc::new(FakeIdGenerator::default());
        let repo = FakeRepo {
            node: Arc::new(Mutex::new(None)),
        };

        let node_id = NodeId::from_str("22222222-2222-2222-2222-222222222222").unwrap();
        let registry =
            PersistentMediaNodeRegistry::new(test_config(), Box::new(repo), id_gen, node_id);

        let node = test_node(node_id, "instance-1");
        registry.register(node, 1000, clock.as_ref()).await.unwrap();
        registry.deregister(node_id, clock.as_ref()).await.unwrap();

        // While within the deregister protection window, the node is still listed.
        let active = registry.list_active(clock.as_ref()).await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, NodeStatus::Left);

        clock.advance_wall(DurationMs::from_millis(120_000));

        let active = registry.list_active(clock.as_ref()).await;
        assert!(active.is_empty());

        // The in-memory cache entry should also be removed, not just filtered.
        let nodes = registry.nodes.read().await;
        assert!(nodes.is_empty());
    }

    #[tokio::test]
    async fn heartbeat_updates_lease_and_load_without_holding_nodes_write_lock() {
        let clock = Arc::new(FakeClock::new());
        let id_gen: Arc<dyn IdGenerator> = Arc::new(FakeIdGenerator::default());
        let repo = FakeRepo {
            node: Arc::new(Mutex::new(None)),
        };

        let node_id = NodeId::from_str("33333333-3333-3333-3333-333333333333").unwrap();
        let registry =
            PersistentMediaNodeRegistry::new(test_config(), Box::new(repo), id_gen, node_id);

        let node = test_node(node_id, "instance-1");
        let registered = registry.register(node, 1000, clock.as_ref()).await.unwrap();
        let lease_id = registered.instance_id.clone();

        clock.advance_wall(DurationMs::from_millis(500));

        let heartbeat = registry
            .heartbeat(
                node_id,
                &lease_id,
                registered.instance_epoch,
                42,
                7,
                clock.as_ref(),
            )
            .await
            .unwrap();
        assert_eq!(heartbeat.load, 42);
        assert_eq!(heartbeat.session_count, 7);
        assert!(heartbeat.lease_until.is_some());
        assert!(heartbeat.last_heartbeat_at.is_some());
    }

    #[tokio::test]
    async fn drain_toggles_draining_flag_and_status() {
        let clock = Arc::new(FakeClock::new());
        let id_gen: Arc<dyn IdGenerator> = Arc::new(FakeIdGenerator::default());
        let repo = FakeRepo {
            node: Arc::new(Mutex::new(None)),
        };

        let node_id = NodeId::from_str("44444444-4444-4444-4444-444444444444").unwrap();
        let registry =
            PersistentMediaNodeRegistry::new(test_config(), Box::new(repo), id_gen, node_id);

        let node = test_node(node_id, "instance-1");
        registry.register(node, 1000, clock.as_ref()).await.unwrap();

        let drained = registry.drain(node_id, true, clock.as_ref()).await.unwrap();
        assert_eq!(drained.status, NodeStatus::Draining);
        assert!(drained.draining);

        let active = registry
            .drain(node_id, false, clock.as_ref())
            .await
            .unwrap();
        assert_eq!(active.status, NodeStatus::Active);
        assert!(!active.draining);
    }
}

#[cfg(test)]
#[path = "persistent_registry_test.rs"]
mod persistent_registry_test;

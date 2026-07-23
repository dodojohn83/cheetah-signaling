//! Cluster node lease and heartbeat service.

use crate::compatibility::CompatibilityMatrix;
use crate::error::NodeLeaseError;
use cheetah_domain::{Clock, ClusterNode, NodeCapacity, NodeLoad};
use cheetah_signal_types::{DurationMs, IdGenerator, NodeId, NodeInstanceId};
use cheetah_storage_api::NodeRepository;
use futures::lock::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// Maximum byte length of a deployment zone string.
const MAX_ZONE_BYTES: usize = 256;
/// Maximum byte length of a node binary version string.
const MAX_NODE_VERSION_BYTES: usize = 128;
/// Maximum number of contract versions reported by a node.
const MAX_NODE_CONTRACT_VERSIONS: usize = 64;
/// Maximum byte length of a contract name in a node's reported versions.
const MAX_NODE_CONTRACT_NAME_BYTES: usize = 128;
/// Maximum byte length of a contract version string reported by a node.
const MAX_NODE_CONTRACT_VERSION_BYTES: usize = 128;

/// Manages the cluster node registration and lease for this process.
pub struct NodeLeaseService {
    repository: Arc<Mutex<dyn NodeRepository>>,
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    this_node: NodeId,
    zone: String,
    version: String,
    lease_duration: DurationMs,
    compatibility: Arc<CompatibilityMatrix>,
    instance_id: Option<NodeInstanceId>,
}

impl NodeLeaseService {
    /// Creates a new lease service for `this_node`.
    pub fn new(
        repository: Arc<Mutex<dyn NodeRepository>>,
        clock: Arc<dyn Clock>,
        id_generator: Arc<dyn IdGenerator>,
        this_node: NodeId,
        zone: impl Into<String>,
        version: impl Into<String>,
        lease_duration: DurationMs,
    ) -> Self {
        Self::with_compatibility(
            repository,
            clock,
            id_generator,
            this_node,
            zone,
            version,
            lease_duration,
            Arc::new(CompatibilityMatrix::default()),
        )
    }

    /// Creates a new lease service with a custom compatibility matrix.
    #[allow(clippy::too_many_arguments)]
    pub fn with_compatibility(
        repository: Arc<Mutex<dyn NodeRepository>>,
        clock: Arc<dyn Clock>,
        id_generator: Arc<dyn IdGenerator>,
        this_node: NodeId,
        zone: impl Into<String>,
        version: impl Into<String>,
        lease_duration: DurationMs,
        compatibility: Arc<CompatibilityMatrix>,
    ) -> Self {
        Self {
            repository,
            clock,
            id_generator,
            this_node,
            zone: zone.into(),
            version: version.into(),
            lease_duration,
            compatibility,
            instance_id: None,
        }
    }

    /// Registers this process as a new instance of `this_node`.
    ///
    /// The new `instance_id` fences any previous incarnation.
    pub async fn register(
        &mut self,
        capacity: NodeCapacity,
        contract_versions: HashMap<String, String>,
    ) -> Result<ClusterNode, NodeLeaseError> {
        if self.zone.len() > MAX_ZONE_BYTES {
            return Err(NodeLeaseError::Domain(
                cheetah_domain::DomainError::invalid_argument(format!(
                    "zone must not exceed {MAX_ZONE_BYTES} bytes"
                )),
            ));
        }
        if self.version.len() > MAX_NODE_VERSION_BYTES {
            return Err(NodeLeaseError::Domain(
                cheetah_domain::DomainError::invalid_argument(format!(
                    "version must not exceed {MAX_NODE_VERSION_BYTES} bytes"
                )),
            ));
        }
        if contract_versions.len() > MAX_NODE_CONTRACT_VERSIONS {
            return Err(NodeLeaseError::Domain(
                cheetah_domain::DomainError::invalid_argument(format!(
                    "contract_versions must not exceed {MAX_NODE_CONTRACT_VERSIONS} entries"
                )),
            ));
        }
        for (name, value) in &contract_versions {
            if name.len() > MAX_NODE_CONTRACT_NAME_BYTES {
                return Err(NodeLeaseError::Domain(
                    cheetah_domain::DomainError::invalid_argument(format!(
                        "contract name must not exceed {MAX_NODE_CONTRACT_NAME_BYTES} bytes"
                    )),
                ));
            }
            if value.len() > MAX_NODE_CONTRACT_VERSION_BYTES {
                return Err(NodeLeaseError::Domain(
                    cheetah_domain::DomainError::invalid_argument(format!(
                        "contract {name:?} version must not exceed {MAX_NODE_CONTRACT_VERSION_BYTES} bytes"
                    )),
                ));
            }
        }

        self.compatibility
            .check(&self.version, &contract_versions)?;

        let instance_id = self.id_generator.generate_node_instance_id();
        let now = self.clock.now_wall();
        let lease_until = now.checked_add(self.lease_duration).ok_or_else(|| {
            cheetah_domain::DomainError::internal("node lease timestamp overflow")
        })?;

        let mut node =
            ClusterNode::new(self.this_node, instance_id, &self.zone, &self.version, now);
        node.lease_until = lease_until;
        node.updated_at = now;
        node.capacity = capacity;
        node.contract_versions = contract_versions;

        self.repository.lock().await.register(node.clone()).await?;

        self.instance_id = Some(instance_id);
        info!(
            node_id = %self.this_node,
            instance_id = %instance_id,
            version = %self.version,
            "registered cluster node"
        );
        Ok(node)
    }

    /// Renews the lease for this node and reports current load.
    pub async fn heartbeat(&self, load: NodeLoad) -> Result<ClusterNode, NodeLeaseError> {
        let instance_id = self.instance_id()?;
        let now = self.clock.now_wall();
        let lease_until = now.checked_add(self.lease_duration).ok_or_else(|| {
            cheetah_domain::DomainError::internal("node lease timestamp overflow")
        })?;

        let node = self
            .repository
            .lock()
            .await
            .heartbeat(self.this_node, instance_id, lease_until, now, load)
            .await
            .map_err(NodeLeaseError::Storage)?;

        match node {
            Some(n) => Ok(n),
            None => {
                warn!(node_id = %self.this_node, "node heartbeat rejected, instance fenced");
                Err(NodeLeaseError::Fenced(self.this_node.to_string()))
            }
        }
    }

    /// Marks this node as draining.
    pub async fn mark_draining(&mut self) -> Result<(), NodeLeaseError> {
        let instance_id = self.instance_id()?;
        let now = self.clock.now_wall();
        let updated = self
            .repository
            .lock()
            .await
            .mark_draining(self.this_node, instance_id, now)
            .await
            .map_err(NodeLeaseError::Storage)?;
        if !updated {
            warn!(node_id = %self.this_node, "node drain rejected, instance fenced");
            return Err(NodeLeaseError::Fenced(self.this_node.to_string()));
        }
        Ok(())
    }

    /// Returns the currently registered node, if any.
    pub async fn registered_node(&self) -> Result<Option<ClusterNode>, NodeLeaseError> {
        self.repository
            .lock()
            .await
            .get(self.this_node)
            .await
            .map_err(NodeLeaseError::Storage)
    }

    fn instance_id(&self) -> Result<NodeInstanceId, NodeLeaseError> {
        self.instance_id
            .ok_or_else(|| NodeLeaseError::NotRegistered(self.this_node.to_string()))
    }
}

impl std::fmt::Debug for NodeLeaseService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeLeaseService")
            .field("this_node", &self.this_node)
            .field("instance_id", &self.instance_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
    use cheetah_signal_types::{Page, PageRequest, UtcTimestamp};
    use cheetah_storage_api::StorageError;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as SyncMutex};

    type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

    struct InMemoryNodeRepository {
        nodes: SyncMutex<HashMap<NodeId, ClusterNode>>,
    }

    impl InMemoryNodeRepository {
        fn new() -> Self {
            Self {
                nodes: SyncMutex::new(HashMap::new()),
            }
        }

        fn lock_nodes(
            &self,
        ) -> Result<std::sync::MutexGuard<'_, HashMap<NodeId, ClusterNode>>, StorageError> {
            self.nodes
                .lock()
                .map_err(|e| StorageError::internal(format!("{e}")))
        }
    }

    #[async_trait::async_trait]
    impl NodeRepository for InMemoryNodeRepository {
        async fn register(&mut self, node: ClusterNode) -> Result<(), StorageError> {
            self.lock_nodes()?.insert(node.node_id, node);
            Ok(())
        }

        async fn heartbeat(
            &mut self,
            node_id: NodeId,
            instance_id: NodeInstanceId,
            lease_until: UtcTimestamp,
            updated_at: UtcTimestamp,
            load: NodeLoad,
        ) -> Result<Option<ClusterNode>, StorageError> {
            let mut nodes = self.lock_nodes()?;
            let Some(node) = nodes.get_mut(&node_id) else {
                return Ok(None);
            };
            if node.instance_id != instance_id {
                return Ok(None);
            }
            node.lease_until = lease_until;
            node.updated_at = updated_at;
            node.load = load;
            Ok(Some(node.clone()))
        }

        async fn get(&self, node_id: NodeId) -> Result<Option<ClusterNode>, StorageError> {
            Ok(self.lock_nodes()?.get(&node_id).cloned())
        }

        async fn list_alive(
            &self,
            now: UtcTimestamp,
            page: PageRequest,
        ) -> Result<Page<ClusterNode>, StorageError> {
            let nodes = self.lock_nodes()?;
            let mut alive: Vec<ClusterNode> = nodes
                .values()
                .filter(|n| n.lease_until > now)
                .cloned()
                .collect();
            alive.sort_by(|a, b| {
                a.updated_at
                    .cmp(&b.updated_at)
                    .then(a.node_id.cmp(&b.node_id))
            });

            let mut start = 0;
            if let Some(cursor_value) = &page.cursor {
                let cursor = cheetah_signal_types::ListCursor::decode(cursor_value)
                    .map_err(|e| StorageError::invalid_argument(format!("invalid cursor: {e}")))?;
                let (updated_at, id) = cursor
                    .parse()
                    .map_err(|e| StorageError::invalid_argument(format!("invalid cursor: {e}")))?;
                let cursor_node_id: NodeId = id.into();
                start = match alive.binary_search_by(|n| {
                    n.updated_at
                        .cmp(&updated_at)
                        .then(n.node_id.cmp(&cursor_node_id))
                }) {
                    Ok(i) => i + 1,
                    Err(i) => i,
                };
            }

            let page_size = page.page_size_as_usize_clamped();
            let end = start.saturating_add(page_size).min(alive.len());
            let has_more = alive.len() > end;
            let items: Vec<ClusterNode> = alive.drain(start..end).collect();
            let next_cursor = if has_more {
                let last = items
                    .last()
                    .ok_or_else(|| StorageError::internal("empty page"))?;
                Some(
                    cheetah_signal_types::ListCursor::new(last.updated_at, last.node_id.as_uuid())
                        .map_err(|e| StorageError::internal(e.to_string()))?
                        .encode()
                        .map_err(|e| StorageError::internal(e.to_string()))?,
                )
            } else {
                None
            };

            let mut result = Page::new(items);
            if let Some(cursor) = next_cursor {
                result = result.with_next_cursor(cursor);
            }
            Ok(result)
        }

        async fn mark_draining(
            &mut self,
            node_id: NodeId,
            instance_id: NodeInstanceId,
            updated_at: UtcTimestamp,
        ) -> Result<bool, StorageError> {
            let mut nodes = self.lock_nodes()?;
            if let Some(node) = nodes.get_mut(&node_id)
                && node.instance_id == instance_id
            {
                node.draining = true;
                node.updated_at = updated_at;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    fn setup() -> (
        NodeLeaseService,
        Arc<InMemoryClock>,
        Arc<InMemoryIdGenerator>,
        Arc<Mutex<InMemoryNodeRepository>>,
    ) {
        let clock = Arc::new(InMemoryClock::new());
        let id_gen = Arc::new(InMemoryIdGenerator::new());
        let repo = Arc::new(Mutex::new(InMemoryNodeRepository::new()));
        let node_id = id_gen.generate_node_id();
        let service = NodeLeaseService::new(
            repo.clone(),
            clock.clone(),
            id_gen.clone(),
            node_id,
            "zone-a",
            "0.1.0",
            DurationMs::from_millis(10_000),
        );
        (service, clock, id_gen, repo)
    }

    #[tokio::test]
    async fn register_creates_node_with_lease() -> TestResult<()> {
        let (mut service, clock, _id_gen, repo) = setup();
        let node = service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await?;
        assert_eq!(node.node_id, service.this_node);
        assert!(node.lease_until > clock.now_wall());

        let stored = repo.lock().await.get(node.node_id).await?;
        assert!(stored.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn heartbeat_extends_lease_and_rejects_fenced_instance() -> TestResult<()> {
        let (mut service, clock, id_gen, repo) = setup();
        service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await?;

        clock.advance(DurationMs::from_millis(5_000));
        let node = service.heartbeat(NodeLoad { devices: 3 }).await?;
        assert_eq!(node.load.devices, 3);

        // Simulate a new instance registering with the same node_id.
        let mut new_service = NodeLeaseService::new(
            repo.clone(),
            clock.clone(),
            id_gen.clone(),
            service.this_node,
            "zone-a",
            "0.1.0",
            DurationMs::from_millis(10_000),
        );
        new_service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await?;

        let result = service.heartbeat(NodeLoad { devices: 4 }).await;
        assert!(matches!(result, Err(NodeLeaseError::Fenced(_))));
        Ok(())
    }

    #[tokio::test]
    async fn mark_draining_sets_flag() -> TestResult<()> {
        let (mut service, _clock, _id_gen, repo) = setup();
        service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await?;

        service.mark_draining().await?;

        let stored = repo.lock().await.get(service.this_node).await?;
        assert!(stored.ok_or("node should exist")?.draining);
        Ok(())
    }

    #[tokio::test]
    async fn mark_draining_rejects_fenced_instance() -> TestResult<()> {
        let (mut service, clock, id_gen, repo) = setup();
        service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await?;

        let mut new_service = NodeLeaseService::new(
            repo.clone(),
            clock.clone(),
            id_gen.clone(),
            service.this_node,
            "zone-a",
            "0.1.0",
            DurationMs::from_millis(10_000),
        );
        new_service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await?;

        let result = service.mark_draining().await;
        assert!(matches!(result, Err(NodeLeaseError::Fenced(_))));
        Ok(())
    }

    #[tokio::test]
    async fn register_rejects_incompatible_version() -> TestResult<()> {
        let clock = Arc::new(InMemoryClock::new());
        let id_gen = Arc::new(InMemoryIdGenerator::new());
        let repo = Arc::new(Mutex::new(InMemoryNodeRepository::new()));
        let node_id = id_gen.generate_node_id();

        let matrix = crate::compatibility::CompatibilityMatrix::new(
            ">=1.0.0, <2.0.0",
            std::collections::HashMap::new(),
        )?;
        let compatibility = Arc::new(matrix);
        let mut service = NodeLeaseService::with_compatibility(
            repo.clone(),
            clock.clone(),
            id_gen.clone(),
            node_id,
            "zone-a",
            "0.5.0",
            DurationMs::from_millis(10_000),
            compatibility,
        );

        let result = service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await;
        assert!(matches!(result, Err(NodeLeaseError::Incompatible(_))));
        Ok(())
    }

    #[tokio::test]
    async fn register_accepts_compatible_version() -> TestResult<()> {
        let clock = Arc::new(InMemoryClock::new());
        let id_gen = Arc::new(InMemoryIdGenerator::new());
        let repo = Arc::new(Mutex::new(InMemoryNodeRepository::new()));
        let node_id = id_gen.generate_node_id();

        let mut contracts = HashMap::new();
        contracts.insert(
            "cheetah.media.v1".to_string(),
            ">=1.0.0, <2.0.0".to_string(),
        );
        let matrix = crate::compatibility::CompatibilityMatrix::new(">=1.0.0, <2.0.0", contracts)?;
        let compatibility = Arc::new(matrix);
        let mut service = NodeLeaseService::with_compatibility(
            repo.clone(),
            clock.clone(),
            id_gen.clone(),
            node_id,
            "zone-a",
            "1.2.0",
            DurationMs::from_millis(10_000),
            compatibility,
        );

        let mut node_contracts = HashMap::new();
        node_contracts.insert("cheetah.media.v1".to_string(), "1.5.0".to_string());
        let node = service
            .register(NodeCapacity { max_devices: 100 }, node_contracts)
            .await?;
        assert_eq!(node.version, "1.2.0");
        Ok(())
    }

    fn oversized_service(zone: &str, version: &str) -> NodeLeaseService {
        let clock = Arc::new(InMemoryClock::new());
        let id_gen = Arc::new(InMemoryIdGenerator::new());
        let repo = Arc::new(Mutex::new(InMemoryNodeRepository::new()));
        let node_id = id_gen.generate_node_id();
        NodeLeaseService::new(
            repo,
            clock,
            id_gen,
            node_id,
            zone.to_string(),
            version.to_string(),
            DurationMs::from_millis(10_000),
        )
    }

    #[tokio::test]
    async fn register_rejects_oversized_zone() -> TestResult<()> {
        let mut service = oversized_service(&"a".repeat(257), "0.1.0");
        let result = service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await;
        assert!(matches!(result, Err(NodeLeaseError::Domain(_))));
        Ok(())
    }

    #[tokio::test]
    async fn register_rejects_oversized_version() -> TestResult<()> {
        let mut service = oversized_service("zone-a", &"1.".repeat(65));
        let result = service
            .register(NodeCapacity { max_devices: 100 }, HashMap::new())
            .await;
        assert!(matches!(result, Err(NodeLeaseError::Domain(_))));
        Ok(())
    }

    #[tokio::test]
    async fn register_rejects_too_many_contract_versions() -> TestResult<()> {
        let mut service = oversized_service("zone-a", "0.1.0");
        let contracts = (0..65)
            .map(|i| (format!("contract-{i}"), "1.0.0".to_string()))
            .collect();
        let result = service
            .register(NodeCapacity { max_devices: 100 }, contracts)
            .await;
        assert!(matches!(result, Err(NodeLeaseError::Domain(_))));
        Ok(())
    }

    #[tokio::test]
    async fn register_rejects_oversized_contract_name() -> TestResult<()> {
        let mut service = oversized_service("zone-a", "0.1.0");
        let contracts = HashMap::from([("x".repeat(129), "1.0.0".to_string())]);
        let result = service
            .register(NodeCapacity { max_devices: 100 }, contracts)
            .await;
        assert!(matches!(result, Err(NodeLeaseError::Domain(_))));
        Ok(())
    }

    #[tokio::test]
    async fn register_rejects_oversized_contract_version() -> TestResult<()> {
        let mut service = oversized_service("zone-a", "0.1.0");
        let contracts = HashMap::from([("cheetah.media.v1".to_string(), "x".repeat(129))]);
        let result = service
            .register(NodeCapacity { max_devices: 100 }, contracts)
            .await;
        assert!(matches!(result, Err(NodeLeaseError::Domain(_))));
        Ok(())
    }
}

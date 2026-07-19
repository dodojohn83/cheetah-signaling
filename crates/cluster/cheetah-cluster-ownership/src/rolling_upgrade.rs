//! Rolling upgrade support: drain a node and migrate its owned devices.

use crate::assignment::DeviceAssignmentService;
use crate::error::RollingUpgradeError;
use cheetah_domain::{Clock, OwnerInfo};
use cheetah_signal_types::{DeviceId, MAX_PAGE_SIZE, NodeId, PageRequest, TenantId};
use cheetah_storage_api::{NodeRepository, OwnerRepository};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Lookup for the protocol of a device during drain-driven migration.
#[async_trait::async_trait]
pub trait DeviceProtocolLookup: Send + Sync {
    /// Returns the protocol identifier for `device_id`, or `None` if the device
    /// no longer exists.
    async fn protocol_for(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Option<String>, RollingUpgradeError>;
}

/// Result of draining a node.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DrainReport {
    /// Devices successfully migrated to another node.
    pub migrated: u64,
    /// Devices that could not be migrated.
    pub failed: u64,
    /// Devices skipped because their protocol could not be determined.
    pub skipped: u64,
    /// Whether more devices remained after hitting the per-drain limit.
    pub truncated: bool,
}

/// Migrates devices off a draining node.
pub struct DrainingMigrationService {
    assignment_service: DeviceAssignmentService,
    owner_repository: Arc<Mutex<dyn OwnerRepository>>,
    node_repository: Arc<Mutex<dyn NodeRepository>>,
    clock: Arc<dyn Clock>,
    batch_size: u32,
    max_devices: u64,
}

impl DrainingMigrationService {
    /// Creates a new draining migration service.
    ///
    /// `max_devices` bounds the number of unique devices attempted in a single
    /// `drain_node` call; `DrainReport::truncated` is set when more devices
    /// remain and the caller should resume draining later.
    pub fn new(
        assignment_service: DeviceAssignmentService,
        owner_repository: Arc<Mutex<dyn OwnerRepository>>,
        node_repository: Arc<Mutex<dyn NodeRepository>>,
        clock: Arc<dyn Clock>,
        batch_size: u32,
        max_devices: u64,
    ) -> Self {
        Self {
            assignment_service,
            owner_repository,
            node_repository,
            clock,
            batch_size: batch_size.clamp(1, MAX_PAGE_SIZE),
            max_devices: max_devices.max(1),
        }
    }

    /// Marks `node_id` as draining and migrates up to `max_devices` devices owned
    /// by that node to other eligible nodes.
    ///
    /// The `lookup` is called for each owned device to determine the protocol
    /// used for node selection. Devices whose protocol is unknown are counted as
    /// skipped. `DrainReport::truncated` is set when the device limit is reached
    /// before all devices are drained.
    pub async fn drain_node(
        &self,
        node_id: NodeId,
        lookup: &dyn DeviceProtocolLookup,
    ) -> Result<DrainReport, RollingUpgradeError> {
        let now = self.clock.now_wall();
        let node = self
            .node_repository
            .lock()
            .await
            .get(node_id)
            .await
            .map_err(RollingUpgradeError::Storage)?
            .ok_or_else(|| RollingUpgradeError::NodeNotFound(node_id.to_string()))?;

        let marked = self
            .node_repository
            .lock()
            .await
            .mark_draining(node_id, node.instance_id, now)
            .await
            .map_err(RollingUpgradeError::Storage)?;
        if !marked {
            return Err(RollingUpgradeError::Fenced(node_id.to_string()));
        }

        info!(node_id = %node_id, "marked node as draining");

        let mut report = DrainReport::default();
        let mut attempted =
            HashSet::<(TenantId, DeviceId)>::with_capacity(self.max_devices as usize);
        let mut processed = 0u64;
        let mut cursor: Option<String> = None;
        'drain: loop {
            let mut request = PageRequest::new(self.batch_size)?;
            request.cursor = cursor;
            let page = self
                .owner_repository
                .lock()
                .await
                .list_by_node(node_id, request)
                .await
                .map_err(RollingUpgradeError::Storage)?;

            for owned in page.items {
                if processed >= self.max_devices {
                    report.truncated = true;
                    break 'drain;
                }
                if !attempted.insert((owned.tenant_id, owned.device_id)) {
                    continue;
                }
                processed += 1;
                match lookup.protocol_for(owned.tenant_id, owned.device_id).await {
                    Ok(Some(protocol)) => {
                        match self
                            .migrate_device(owned.tenant_id, owned.device_id, &protocol, node_id)
                            .await
                        {
                            Ok(_) => report.migrated += 1,
                            Err(e) => {
                                report.failed += 1;
                                warn!(
                                    tenant_id = %owned.tenant_id.as_uuid(),
                                    device_id = %owned.device_id.as_uuid(),
                                    node_id = %node_id.as_uuid(),
                                    "failed to migrate device off draining node: {e}"
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        report.skipped += 1;
                        warn!(
                            tenant_id = %owned.tenant_id.as_uuid(),
                            device_id = %owned.device_id.as_uuid(),
                            "skipping drain migration for device with unknown protocol"
                        );
                    }
                    Err(e) => {
                        report.failed += 1;
                        warn!(
                            tenant_id = %owned.tenant_id.as_uuid(),
                            device_id = %owned.device_id.as_uuid(),
                            "failed to look up device protocol during drain: {e}"
                        );
                    }
                }
            }

            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        info!(
            node_id = %node_id,
            migrated = report.migrated,
            failed = report.failed,
            skipped = report.skipped,
            "finished draining node"
        );
        Ok(report)
    }

    /// Migrates a single device off `from_node`.
    ///
    /// `from_node` must be draining. The current owner is cleared and a new
    /// eligible node is selected by `DeviceAssignmentService`.
    pub async fn migrate_device(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        protocol: &str,
        from_node: NodeId,
    ) -> Result<OwnerInfo, RollingUpgradeError> {
        let node = self
            .node_repository
            .lock()
            .await
            .get(from_node)
            .await
            .map_err(RollingUpgradeError::Storage)?
            .ok_or_else(|| RollingUpgradeError::NodeNotFound(from_node.to_string()))?;
        if !node.draining {
            return Err(RollingUpgradeError::NotDraining(from_node.to_string()));
        }

        let current = self
            .owner_repository
            .lock()
            .await
            .get(tenant_id, device_id)
            .await
            .map_err(RollingUpgradeError::Storage)?;
        if let Some(ref owner) = current
            && owner.owner_node_id != from_node
        {
            return Ok(owner.clone());
        }

        let previous = current.clone();
        if previous.is_some() {
            self.owner_repository
                .lock()
                .await
                .clear(tenant_id, device_id)
                .await
                .map_err(RollingUpgradeError::Storage)?;
        }

        match self
            .assignment_service
            .assign(tenant_id, device_id, protocol, None)
            .await
        {
            Ok(owner) => Ok(owner),
            Err(e) => {
                if let Some(owner) = previous {
                    if let Err(restore_err) = self
                        .owner_repository
                        .lock()
                        .await
                        .set(tenant_id, device_id, owner)
                        .await
                    {
                        warn!(
                            tenant_id = %tenant_id.as_uuid(),
                            device_id = %device_id.as_uuid(),
                            node_id = %from_node.as_uuid(),
                            "failed to restore owner after failed migration: {restore_err}"
                        );
                    } else {
                        warn!(
                            tenant_id = %tenant_id.as_uuid(),
                            device_id = %device_id.as_uuid(),
                            node_id = %from_node.as_uuid(),
                            "restored owner on draining node after reassignment failure"
                        );
                    }
                }
                Err(RollingUpgradeError::Assignment(e))
            }
        }
    }
}

impl std::fmt::Debug for DrainingMigrationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrainingMigrationService")
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignment::{DeviceAssignmentService, RateLimitConfig};
    use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
    use cheetah_domain::{NodeCapacity, NodeLoad, OwnerInfo};
    use cheetah_signal_types::{
        DeviceId, DurationMs, IdGenerator, NodeId, NodeInstanceId, OwnerEpoch, Page, PageRequest,
        TenantId, UtcTimestamp,
    };
    use cheetah_storage_api::{NodeRepository, OwnedDevice, OwnerRepository, StorageError};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex as AsyncMutex;

    type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

    fn make_node(
        id_gen: &InMemoryIdGenerator,
        zone: &str,
        protocols: &[&str],
        load: u32,
        capacity: u32,
        lease_until: UtcTimestamp,
    ) -> cheetah_domain::ClusterNode {
        let instance_id = id_gen.generate_node_instance_id();
        let node_id = id_gen.generate_node_id();
        let mut node =
            cheetah_domain::ClusterNode::new(node_id, instance_id, zone, "0.1.0", lease_until);
        node.lease_until = lease_until;
        node.updated_at = lease_until;
        node.capacity = NodeCapacity {
            max_devices: capacity,
        };
        node.load = NodeLoad { devices: load };
        node.contract_versions = protocols
            .iter()
            .map(|p| (p.to_string(), "1".to_string()))
            .collect();
        node
    }

    fn wall_ms(ts: UtcTimestamp) -> u64 {
        let offset = ts.as_offset();
        (offset.unix_timestamp() * 1_000 + i64::from(offset.nanosecond()) / 1_000_000) as u64
    }

    struct FakeNodeRepository {
        nodes: StdMutex<HashMap<NodeId, cheetah_domain::ClusterNode>>,
    }

    impl FakeNodeRepository {
        fn new() -> Self {
            Self {
                nodes: StdMutex::new(HashMap::new()),
            }
        }

        fn nodes(&self) -> std::sync::MutexGuard<'_, HashMap<NodeId, cheetah_domain::ClusterNode>> {
            match self.nodes.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            }
        }

        fn insert(&self, node: cheetah_domain::ClusterNode) {
            self.nodes().insert(node.node_id, node);
        }
    }

    #[async_trait::async_trait]
    impl NodeRepository for FakeNodeRepository {
        async fn register(
            &mut self,
            _node: cheetah_domain::ClusterNode,
        ) -> Result<(), StorageError> {
            unimplemented!("not used in tests")
        }

        async fn heartbeat(
            &mut self,
            _node_id: NodeId,
            _instance_id: NodeInstanceId,
            _lease_until: UtcTimestamp,
            _updated_at: UtcTimestamp,
            _load: NodeLoad,
        ) -> Result<Option<cheetah_domain::ClusterNode>, StorageError> {
            unimplemented!("not used in tests")
        }

        async fn get(
            &self,
            node_id: NodeId,
        ) -> Result<Option<cheetah_domain::ClusterNode>, StorageError> {
            Ok(self.nodes().get(&node_id).cloned())
        }

        async fn list_alive(
            &self,
            now: UtcTimestamp,
            _page: PageRequest,
        ) -> Result<Page<cheetah_domain::ClusterNode>, StorageError> {
            let nodes: Vec<cheetah_domain::ClusterNode> = self
                .nodes()
                .values()
                .filter(|n| n.lease_until > now)
                .cloned()
                .collect();
            Ok(Page::new(nodes))
        }

        async fn mark_draining(
            &mut self,
            node_id: NodeId,
            instance_id: NodeInstanceId,
            _updated_at: UtcTimestamp,
        ) -> Result<bool, StorageError> {
            let mut nodes = self.nodes();
            if let Some(node) = nodes.get_mut(&node_id)
                && node.instance_id == instance_id
            {
                node.draining = true;
                return Ok(true);
            }
            Ok(false)
        }
    }

    struct FakeOwnerRepository {
        owners: StdMutex<HashMap<(TenantId, DeviceId), OwnerInfo>>,
        next_epoch: AtomicU64,
    }

    impl FakeOwnerRepository {
        fn new() -> Self {
            Self {
                owners: StdMutex::new(HashMap::new()),
                next_epoch: AtomicU64::new(2),
            }
        }

        fn owners(&self) -> std::sync::MutexGuard<'_, HashMap<(TenantId, DeviceId), OwnerInfo>> {
            match self.owners.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            }
        }
    }

    #[async_trait::async_trait]
    impl OwnerRepository for FakeOwnerRepository {
        async fn get(
            &self,
            tenant_id: TenantId,
            device_id: DeviceId,
        ) -> Result<Option<OwnerInfo>, StorageError> {
            Ok(self.owners().get(&(tenant_id, device_id)).cloned())
        }

        async fn set(
            &mut self,
            tenant_id: TenantId,
            device_id: DeviceId,
            owner: OwnerInfo,
        ) -> Result<(), StorageError> {
            self.owners().insert((tenant_id, device_id), owner);
            Ok(())
        }

        async fn clear(
            &mut self,
            tenant_id: TenantId,
            device_id: DeviceId,
        ) -> Result<(), StorageError> {
            self.owners().remove(&(tenant_id, device_id));
            Ok(())
        }

        async fn acquire(
            &mut self,
            tenant_id: TenantId,
            device_id: DeviceId,
            node_id: NodeId,
            now: UtcTimestamp,
            lease_until: UtcTimestamp,
        ) -> Result<OwnerInfo, StorageError> {
            let mut owners = self.owners();
            let key = (tenant_id, device_id);
            if let Some(owner) = owners.get(&key) {
                if let Some(lease) = owner.lease_until
                    && lease > now
                    && owner.owner_node_id != node_id
                {
                    return Err(StorageError::unavailable(
                        "device lease held by another node",
                    ));
                }
                if owner.owner_node_id == node_id {
                    let updated = OwnerInfo {
                        owner_node_id: node_id,
                        owner_epoch: owner.owner_epoch,
                        lease_until: Some(lease_until),
                    };
                    owners.insert(key, updated.clone());
                    return Ok(updated);
                }
            }
            let epoch = OwnerEpoch(self.next_epoch.fetch_add(1, Ordering::SeqCst));
            let owner = OwnerInfo {
                owner_node_id: node_id,
                owner_epoch: epoch,
                lease_until: Some(lease_until),
            };
            owners.insert(key, owner.clone());
            Ok(owner)
        }

        async fn renew(
            &mut self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
            _node_id: NodeId,
            _lease_until: UtcTimestamp,
        ) -> Result<Option<OwnerInfo>, StorageError> {
            unimplemented!("not used in tests")
        }

        async fn release(
            &mut self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
            _node_id: NodeId,
            _epoch: OwnerEpoch,
        ) -> Result<(), StorageError> {
            unimplemented!("not used in tests")
        }

        async fn list_by_node(
            &self,
            node_id: NodeId,
            _page: PageRequest,
        ) -> Result<Page<OwnedDevice>, StorageError> {
            let owners = self.owners();
            let items: Vec<OwnedDevice> = owners
                .iter()
                .filter(|(_, o)| o.owner_node_id == node_id)
                .map(|((tenant_id, device_id), owner)| OwnedDevice {
                    tenant_id: *tenant_id,
                    device_id: *device_id,
                    owner: owner.clone(),
                })
                .collect();
            Ok(Page::new(items))
        }
    }

    struct FakeLookup {
        protocols: StdMutex<HashMap<(TenantId, DeviceId), String>>,
    }

    #[async_trait::async_trait]
    impl DeviceProtocolLookup for FakeLookup {
        async fn protocol_for(
            &self,
            tenant_id: TenantId,
            device_id: DeviceId,
        ) -> Result<Option<String>, RollingUpgradeError> {
            Ok(self
                .protocols
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&(tenant_id, device_id))
                .cloned())
        }
    }

    #[allow(clippy::type_complexity)]
    fn setup() -> (
        DrainingMigrationService,
        Arc<InMemoryClock>,
        Arc<AsyncMutex<FakeNodeRepository>>,
        Arc<AsyncMutex<FakeOwnerRepository>>,
        InMemoryIdGenerator,
    ) {
        let clock = Arc::new(InMemoryClock::new());
        let node_repo = Arc::new(AsyncMutex::new(FakeNodeRepository::new()));
        let owner_repo = Arc::new(AsyncMutex::new(FakeOwnerRepository::new()));
        let assignment_service = DeviceAssignmentService::new(
            node_repo.clone(),
            owner_repo.clone(),
            clock.clone(),
            DurationMs::from_millis(60_000),
            RateLimitConfig::default(),
        );
        let service = DrainingMigrationService::new(
            assignment_service,
            owner_repo.clone(),
            node_repo.clone(),
            clock.clone(),
            100,
            1_000,
        );
        let id_gen = InMemoryIdGenerator::new();
        (service, clock, node_repo, owner_repo, id_gen)
    }

    #[tokio::test]
    async fn migrate_device_moves_off_draining_node() -> TestResult<()> {
        let (service, clock, node_repo, owner_repo, id_gen) = setup();
        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;

        let mut draining_node = make_node(&id_gen, "zone-a", &["gb28181"], 1, 10, lease);
        draining_node.draining = true;
        let draining_id = draining_node.node_id;
        let spare_node = make_node(&id_gen, "zone-a", &["gb28181"], 0, 10, lease);
        let spare_id = spare_node.node_id;
        node_repo.lock().await.insert(draining_node);
        node_repo.lock().await.insert(spare_node);

        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        owner_repo
            .lock()
            .await
            .set(
                tenant,
                device,
                OwnerInfo {
                    owner_node_id: draining_id,
                    owner_epoch: OwnerEpoch(1),
                    lease_until: Some(lease),
                },
            )
            .await?;

        let owner = service
            .migrate_device(tenant, device, "gb28181", draining_id)
            .await?;
        assert_eq!(owner.owner_node_id, spare_id);
        assert!(owner.owner_node_id != draining_id);
        Ok(())
    }

    #[tokio::test]
    async fn drain_node_migrates_all_devices() -> TestResult<()> {
        let (service, clock, node_repo, owner_repo, id_gen) = setup();
        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;

        let draining_node = make_node(&id_gen, "zone-a", &["gb28181"], 1, 10, lease);
        let draining_id = draining_node.node_id;
        let spare_node = make_node(&id_gen, "zone-a", &["gb28181"], 0, 10, lease);
        let spare_id = spare_node.node_id;
        node_repo.lock().await.insert(draining_node);
        node_repo.lock().await.insert(spare_node);

        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        owner_repo
            .lock()
            .await
            .set(
                tenant,
                device,
                OwnerInfo {
                    owner_node_id: draining_id,
                    owner_epoch: OwnerEpoch(1),
                    lease_until: Some(lease),
                },
            )
            .await?;

        let lookup = FakeLookup {
            protocols: {
                let mut map = HashMap::new();
                map.insert((tenant, device), "gb28181".to_string());
                StdMutex::new(map)
            },
        };

        let report = service.drain_node(draining_id, &lookup).await?;
        assert_eq!(report.migrated, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(report.skipped, 0);

        let owner = owner_repo.lock().await.get(tenant, device).await?;
        assert!(owner.is_some_and(|o| o.owner_node_id == spare_id));
        Ok(())
    }

    #[tokio::test]
    async fn failed_migration_preserves_owner() -> TestResult<()> {
        let (service, clock, node_repo, owner_repo, id_gen) = setup();
        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;

        let mut draining_node = make_node(&id_gen, "zone-a", &["gb28181"], 1, 10, lease);
        draining_node.draining = true;
        let draining_id = draining_node.node_id;
        node_repo.lock().await.insert(draining_node);

        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        let original = OwnerInfo {
            owner_node_id: draining_id,
            owner_epoch: OwnerEpoch(7),
            lease_until: Some(lease),
        };
        owner_repo
            .lock()
            .await
            .set(tenant, device, original.clone())
            .await?;

        let result = service
            .migrate_device(tenant, device, "gb28181", draining_id)
            .await;
        assert!(matches!(result, Err(RollingUpgradeError::Assignment(_))));

        let owner = owner_repo.lock().await.get(tenant, device).await?;
        assert_eq!(owner, Some(original));
        Ok(())
    }

    #[tokio::test]
    async fn migrate_device_fails_when_node_not_draining() -> TestResult<()> {
        let (service, clock, node_repo, owner_repo, id_gen) = setup();
        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;

        let node = make_node(&id_gen, "zone-a", &["gb28181"], 0, 10, lease);
        let node_id = node.node_id;
        node_repo.lock().await.insert(node);

        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        owner_repo
            .lock()
            .await
            .set(
                tenant,
                device,
                OwnerInfo {
                    owner_node_id: node_id,
                    owner_epoch: OwnerEpoch(1),
                    lease_until: Some(lease),
                },
            )
            .await?;

        let result = service
            .migrate_device(tenant, device, "gb28181", node_id)
            .await;
        assert!(matches!(result, Err(RollingUpgradeError::NotDraining(_))));
        Ok(())
    }

    /// Owner repository that simulates a device re-appearing on a later page
    /// after a failed migration (because `set` bumped its `updated_at`).
    /// The first page returns the device with a cursor; the second page returns
    /// the same device with no cursor.
    struct ReStampOwnerRepository {
        owner: AsyncMutex<Option<OwnerInfo>>,
        tenant_device: AsyncMutex<Option<(TenantId, DeviceId)>>,
        calls: AtomicU64,
    }

    impl ReStampOwnerRepository {
        fn new(tenant: TenantId, device: DeviceId, owner: OwnerInfo) -> Self {
            Self {
                owner: AsyncMutex::new(Some(owner)),
                tenant_device: AsyncMutex::new(Some((tenant, device))),
                calls: AtomicU64::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl OwnerRepository for ReStampOwnerRepository {
        async fn get(
            &self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
        ) -> Result<Option<OwnerInfo>, StorageError> {
            Ok(self.owner.lock().await.clone())
        }

        async fn set(
            &mut self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
            owner: OwnerInfo,
        ) -> Result<(), StorageError> {
            *self.owner.lock().await = Some(owner);
            Ok(())
        }

        async fn clear(
            &mut self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
        ) -> Result<(), StorageError> {
            *self.owner.lock().await = None;
            Ok(())
        }

        async fn acquire(
            &mut self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
            _node_id: NodeId,
            _now: UtcTimestamp,
            _lease_until: UtcTimestamp,
        ) -> Result<OwnerInfo, StorageError> {
            unimplemented!("not used in this test")
        }

        async fn renew(
            &mut self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
            _node_id: NodeId,
            _lease_until: UtcTimestamp,
        ) -> Result<Option<OwnerInfo>, StorageError> {
            unimplemented!("not used in this test")
        }

        async fn release(
            &mut self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
            _node_id: NodeId,
            _epoch: OwnerEpoch,
        ) -> Result<(), StorageError> {
            unimplemented!("not used in this test")
        }

        async fn list_by_node(
            &self,
            _node_id: NodeId,
            _page: PageRequest,
        ) -> Result<Page<OwnedDevice>, StorageError> {
            let calls = self.calls.fetch_add(1, Ordering::SeqCst);
            let guard = self.tenant_device.lock().await;
            let owner = self.owner.lock().await.clone();
            let Some((tenant_id, device_id)) = *guard else {
                return Ok(Page::new(vec![]));
            };
            let Some(owner) = owner else {
                return Ok(Page::new(vec![]));
            };
            let mut page = Page::new(vec![OwnedDevice {
                tenant_id,
                device_id,
                owner,
            }]);
            if calls == 0 {
                page = page.with_next_cursor("more");
            }
            Ok(page)
        }
    }

    #[tokio::test]
    async fn drain_node_terminates_on_repeated_device() -> TestResult<()> {
        let clock = Arc::new(InMemoryClock::new());
        let id_gen = InMemoryIdGenerator::new();
        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;

        let mut draining_node = make_node(&id_gen, "zone-a", &["gb28181"], 1, 10, lease);
        draining_node.draining = true;
        let draining_id = draining_node.node_id;
        let node_repo = Arc::new(AsyncMutex::new(FakeNodeRepository::new()));
        node_repo.lock().await.insert(draining_node);

        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        let owner = OwnerInfo {
            owner_node_id: draining_id,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(lease),
        };
        let owner_repo = Arc::new(AsyncMutex::new(ReStampOwnerRepository::new(
            tenant, device, owner,
        )));
        let assignment_service = DeviceAssignmentService::new(
            node_repo.clone(),
            owner_repo.clone(),
            clock.clone(),
            DurationMs::from_millis(60_000),
            RateLimitConfig::default(),
        );
        let service = DrainingMigrationService::new(
            assignment_service,
            owner_repo.clone(),
            node_repo.clone(),
            clock.clone(),
            100,
            1_000,
        );

        let lookup = FakeLookup {
            protocols: {
                let mut map = HashMap::new();
                map.insert((tenant, device), "gb28181".to_string());
                StdMutex::new(map)
            },
        };

        let report = service.drain_node(draining_id, &lookup).await?;
        assert_eq!(report.migrated, 0);
        assert_eq!(report.failed, 1);

        let owner = owner_repo.lock().await.get(tenant, device).await?;
        assert_eq!(owner.map(|o| o.owner_node_id), Some(draining_id));
        Ok(())
    }
}

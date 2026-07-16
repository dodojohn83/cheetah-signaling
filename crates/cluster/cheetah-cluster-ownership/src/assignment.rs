//! Cluster-wide device assignment service.

use crate::error::DeviceAssignmentError;
use cheetah_domain::{Clock, ClusterNode};
use cheetah_signal_types::{
    DeviceId, DurationMs, MAX_PAGE_SIZE, NodeId, PageRequest, TenantId, UtcTimestamp,
};
use cheetah_storage_api::{NodeRepository, OwnerRepository, StorageError};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

/// Assigns devices to cluster nodes while preserving existing owners when
/// possible and respecting node health, zone, protocol support and load.
pub struct DeviceAssignmentService {
    node_repository: Arc<tokio::sync::Mutex<dyn NodeRepository>>,
    owner_repository: Arc<tokio::sync::Mutex<dyn OwnerRepository>>,
    clock: Arc<dyn Clock>,
    lease_duration: DurationMs,
    rate_limiter: Mutex<RateLimiter>,
}

impl DeviceAssignmentService {
    /// Creates a new assignment service.
    pub fn new(
        node_repository: Arc<tokio::sync::Mutex<dyn NodeRepository>>,
        owner_repository: Arc<tokio::sync::Mutex<dyn OwnerRepository>>,
        clock: Arc<dyn Clock>,
        lease_duration: DurationMs,
        rate_limit: RateLimitConfig,
    ) -> Self {
        Self {
            node_repository,
            owner_repository,
            clock,
            lease_duration,
            rate_limiter: Mutex::new(RateLimiter::new(rate_limit)),
        }
    }

    /// Assigns (or resolves) the owner for `device_id`.
    ///
    /// If the device already has a valid owner on an eligible node, that owner
    /// is returned without changing the assignment. Otherwise a new node is
    /// selected using a stable hash and acquired through `OwnerRepository`.
    pub async fn assign(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        protocol: &str,
        preferred_zone: Option<&str>,
    ) -> Result<cheetah_domain::OwnerInfo, DeviceAssignmentError> {
        let now = self.clock.now_wall();
        let lease_until = now
            .checked_add(self.lease_duration)
            .ok_or_else(|| cheetah_domain::DomainError::internal("owner lease overflow"))?;

        // Fast path: keep the current owner if its lease is still valid and the
        // node is alive and eligible. Drop the owner lock before looking up the
        // node to avoid holding two async mutex guards across an await.
        let maybe_owner = {
            let owner_repo = self.owner_repository.lock().await;
            owner_repo.get(tenant_id, device_id).await?
        };
        if let Some(owner) = maybe_owner
            && owner.lease_until.is_none_or(|lease| lease > now)
            && let Some(node) = self.get_node(owner.owner_node_id).await?
            && is_eligible(&node, protocol, now)
        {
            return Ok(owner);
        }

        // Select the best candidate from the live cluster view.
        let candidate = self
            .select_node(tenant_id, device_id, protocol, preferred_zone, now)
            .await?;

        // Enforce per-node and global assignment rate limits.
        {
            let mut limiter = self.rate_limiter.lock().map_err(|e| {
                StorageError::internal(format!("assignment rate limiter poisoned: {e}"))
            })?;
            let now_mono = self.clock.now_monotonic();
            if !limiter.check_and_record(now_mono, candidate.node_id) {
                return Err(DeviceAssignmentError::RateLimited);
            }
        }

        // Acquire ownership on the selected node. If another node won the
        // race, return the owner it established.
        let mut owner_repo = self.owner_repository.lock().await;
        match owner_repo
            .acquire(tenant_id, device_id, candidate.node_id, lease_until)
            .await
        {
            Ok(owner) => {
                info!(
                    tenant_id = %tenant_id.as_uuid(),
                    device_id = %device_id.as_uuid(),
                    node_id = %candidate.node_id.as_uuid(),
                    epoch = owner.owner_epoch.0,
                    "assigned device to node"
                );
                Ok(owner)
            }
            Err(ref e) if matches!(e, StorageError::Unavailable { .. }) => {
                if let Some(owner) = owner_repo.get(tenant_id, device_id).await? {
                    warn!(
                        tenant_id = %tenant_id.as_uuid(),
                        device_id = %device_id.as_uuid(),
                        "assignment race lost, returning current owner"
                    );
                    Ok(owner)
                } else {
                    Err(DeviceAssignmentError::NoAvailableNode)
                }
            }
            Err(e) => Err(DeviceAssignmentError::Storage(e)),
        }
    }

    async fn get_node(&self, node_id: NodeId) -> Result<Option<ClusterNode>, StorageError> {
        self.node_repository.lock().await.get(node_id).await
    }

    async fn select_node(
        &self,
        _tenant_id: TenantId,
        device_id: DeviceId,
        protocol: &str,
        preferred_zone: Option<&str>,
        now: UtcTimestamp,
    ) -> Result<ClusterNode, DeviceAssignmentError> {
        let mut candidates = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut req = PageRequest::new(MAX_PAGE_SIZE)?;
            if let Some(c) = &cursor {
                req = req.with_cursor(c.clone());
            }
            let page = self
                .node_repository
                .lock()
                .await
                .list_alive(now, req)
                .await?;
            for node in page.items {
                if is_eligible(&node, protocol, now) {
                    candidates.push(node);
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        if let Some(zone) = preferred_zone {
            let preferred: Vec<ClusterNode> = candidates
                .iter()
                .filter(|n| n.zone == zone)
                .cloned()
                .collect();
            if !preferred.is_empty() {
                candidates = preferred;
            }
        }

        if candidates.is_empty() {
            return Err(DeviceAssignmentError::NoAvailableNode);
        }

        let mut scored: Vec<(u64, ClusterNode)> = candidates
            .into_iter()
            .map(|n| (hash_score(device_id, n.node_id), n))
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.node_id.cmp(&b.1.node_id)));
        let (_, candidate) = scored
            .into_iter()
            .next()
            .ok_or(DeviceAssignmentError::NoAvailableNode)?;
        Ok(candidate)
    }
}

impl std::fmt::Debug for DeviceAssignmentService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceAssignmentService")
            .finish_non_exhaustive()
    }
}

/// Configuration for assignment rate limiting.
#[derive(Clone, Copy, Debug)]
pub struct RateLimitConfig {
    /// Maximum number of new assignments per second across all nodes.
    pub global_per_second: u32,
    /// Maximum number of new assignments per second for a single node.
    pub per_node_per_second: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            global_per_second: 1_000,
            per_node_per_second: 100,
        }
    }
}

struct SlidingWindow {
    max_per_window: u32,
    window: DurationMs,
    events: VecDeque<DurationMs>,
}

impl SlidingWindow {
    fn new(max_per_window: u32, window: DurationMs) -> Self {
        Self {
            max_per_window,
            window,
            events: VecDeque::new(),
        }
    }

    fn prune(&mut self, now: DurationMs) {
        while let Some(front) = self.events.front() {
            if now.as_millis().saturating_sub(front.as_millis()) > self.window.as_millis() {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    fn allow(&mut self, now: DurationMs) -> bool {
        self.prune(now);
        self.events.len() < self.max_per_window as usize
    }

    fn record(&mut self, now: DurationMs) {
        self.events.push_back(now);
    }
}

struct RateLimiter {
    global: SlidingWindow,
    per_node: HashMap<NodeId, SlidingWindow>,
    config: RateLimitConfig,
}

impl RateLimiter {
    fn new(config: RateLimitConfig) -> Self {
        let window = DurationMs::from_millis(1_000);
        Self {
            global: SlidingWindow::new(config.global_per_second, window),
            per_node: HashMap::new(),
            config,
        }
    }

    fn check_and_record(&mut self, now: DurationMs, node_id: NodeId) -> bool {
        if !self.global.allow(now) {
            return false;
        }
        let node_window = self.per_node.entry(node_id).or_insert_with(|| {
            SlidingWindow::new(
                self.config.per_node_per_second,
                DurationMs::from_millis(1_000),
            )
        });
        if !node_window.allow(now) {
            return false;
        }
        self.global.record(now);
        node_window.record(now);
        true
    }
}

fn is_eligible(node: &ClusterNode, protocol: &str, now: UtcTimestamp) -> bool {
    if node.draining {
        return false;
    }
    if node.lease_until <= now {
        return false;
    }
    if !protocol.is_empty() && !node.contract_versions.contains_key(protocol) {
        return false;
    }
    if node.load.devices >= node.capacity.max_devices {
        return false;
    }
    true
}

fn hash_score(device_id: DeviceId, node_id: NodeId) -> u64 {
    let mut hash = 0xcbf29ce484222325u64; // FNV-1a 64-bit offset basis
    const PRIME: u64 = 0x100000001b3;

    for byte in device_id.as_uuid().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    for byte in node_id.as_uuid().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }

    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
    use cheetah_domain::{NodeCapacity, NodeLoad, OwnerInfo};
    use cheetah_signal_types::{IdGenerator, NodeInstanceId, OwnerEpoch, Page};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex as AsyncMutex;

    type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

    fn wall_ms(ts: UtcTimestamp) -> u64 {
        let offset = ts.as_offset();
        (offset.unix_timestamp() * 1_000 + i64::from(offset.nanosecond()) / 1_000_000) as u64
    }

    struct FakeNodeRepository {
        nodes: StdMutex<HashMap<NodeId, ClusterNode>>,
    }

    impl FakeNodeRepository {
        fn new() -> Self {
            Self {
                nodes: StdMutex::new(HashMap::new()),
            }
        }

        fn nodes(&self) -> std::sync::MutexGuard<'_, HashMap<NodeId, ClusterNode>> {
            match self.nodes.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            }
        }

        fn insert(&self, node: ClusterNode) {
            self.nodes().insert(node.node_id, node);
        }
    }

    #[async_trait::async_trait]
    impl NodeRepository for FakeNodeRepository {
        async fn register(&mut self, _node: ClusterNode) -> Result<(), StorageError> {
            unimplemented!("not used in tests")
        }

        async fn heartbeat(
            &mut self,
            _node_id: NodeId,
            _instance_id: NodeInstanceId,
            _lease_until: UtcTimestamp,
            _updated_at: UtcTimestamp,
            _load: NodeLoad,
        ) -> Result<Option<ClusterNode>, StorageError> {
            unimplemented!("not used in tests")
        }

        async fn get(&self, node_id: NodeId) -> Result<Option<ClusterNode>, StorageError> {
            Ok(self.nodes().get(&node_id).cloned())
        }

        async fn list_alive(
            &self,
            now: UtcTimestamp,
            _page: PageRequest,
        ) -> Result<Page<ClusterNode>, StorageError> {
            let nodes: Vec<ClusterNode> = self
                .nodes()
                .values()
                .filter(|n| n.lease_until > now)
                .cloned()
                .collect();
            Ok(Page::new(nodes))
        }

        async fn mark_draining(
            &mut self,
            _node_id: NodeId,
            _instance_id: NodeInstanceId,
            _updated_at: UtcTimestamp,
        ) -> Result<bool, StorageError> {
            unimplemented!("not used in tests")
        }
    }

    struct FakeOwnerRepository {
        owners: StdMutex<HashMap<(TenantId, DeviceId), OwnerInfo>>,
        next_epoch: AtomicU64,
        clock: Arc<InMemoryClock>,
    }

    impl FakeOwnerRepository {
        fn new(clock: Arc<InMemoryClock>) -> Self {
            Self {
                owners: StdMutex::new(HashMap::new()),
                next_epoch: AtomicU64::new(2),
                clock,
            }
        }
    }

    impl FakeOwnerRepository {
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
            lease_until: UtcTimestamp,
        ) -> Result<OwnerInfo, StorageError> {
            let now = self.clock.now_wall();
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
    }

    fn make_node(
        id_gen: &InMemoryIdGenerator,
        zone: &str,
        protocols: &[&str],
        load: u32,
        capacity: u32,
        lease_until: UtcTimestamp,
    ) -> ClusterNode {
        let instance_id = id_gen.generate_node_instance_id();
        let node_id = id_gen.generate_node_id();
        let mut node = ClusterNode::new(node_id, instance_id, zone, "0.1.0", lease_until);
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

    #[allow(clippy::type_complexity)]
    fn setup() -> (
        DeviceAssignmentService,
        Arc<InMemoryClock>,
        Arc<AsyncMutex<FakeNodeRepository>>,
        Arc<AsyncMutex<FakeOwnerRepository>>,
        InMemoryIdGenerator,
    ) {
        let clock = Arc::new(InMemoryClock::new());
        let node_repo = Arc::new(AsyncMutex::new(FakeNodeRepository::new()));
        let owner_repo = Arc::new(AsyncMutex::new(FakeOwnerRepository::new(clock.clone())));
        let id_gen = InMemoryIdGenerator::new();
        let service = DeviceAssignmentService::new(
            node_repo.clone(),
            owner_repo.clone(),
            clock.clone(),
            DurationMs::from_millis(60_000),
            RateLimitConfig::default(),
        );
        (service, clock, node_repo, owner_repo, id_gen)
    }

    #[tokio::test]
    async fn assigns_to_first_eligible_node() -> TestResult<()> {
        let (service, clock, node_repo, _owner_repo, id_gen) = setup();
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
        let owner = service.assign(tenant, device, "gb28181", None).await?;
        assert_eq!(owner.owner_node_id, node_id);
        Ok(())
    }

    #[tokio::test]
    async fn keeps_existing_owner_when_eligible() -> TestResult<()> {
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
        let existing = OwnerInfo {
            owner_node_id: node_id,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(lease),
        };
        owner_repo
            .lock()
            .await
            .set(tenant, device, existing)
            .await?;

        let owner = service.assign(tenant, device, "gb28181", None).await?;
        assert_eq!(owner.owner_node_id, node_id);
        assert_eq!(owner.owner_epoch.0, 1);
        Ok(())
    }

    #[tokio::test]
    async fn reassigns_when_existing_owner_expired() -> TestResult<()> {
        let (service, clock, node_repo, owner_repo, id_gen) = setup();
        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;
        let old_node = make_node(&id_gen, "zone-a", &["gb28181"], 0, 10, now);
        let new_node = make_node(&id_gen, "zone-b", &["gb28181"], 0, 10, lease);
        node_repo.lock().await.insert(old_node.clone());
        node_repo.lock().await.insert(new_node.clone());

        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        let existing = OwnerInfo {
            owner_node_id: old_node.node_id,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(now),
        };
        owner_repo
            .lock()
            .await
            .set(tenant, device, existing)
            .await?;

        clock.advance(DurationMs::from_millis(1));
        let owner = service.assign(tenant, device, "gb28181", None).await?;
        assert_eq!(owner.owner_node_id, new_node.node_id);
        assert!(owner.owner_epoch.0 > 1);
        Ok(())
    }

    #[tokio::test]
    async fn filters_by_protocol() -> TestResult<()> {
        let (service, clock, node_repo, _owner_repo, id_gen) = setup();
        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;
        let gb_node = make_node(&id_gen, "zone-a", &["gb28181"], 0, 10, lease);
        let gb_node_id = gb_node.node_id;
        let onvif_node = make_node(&id_gen, "zone-a", &["onvif"], 0, 10, lease);
        node_repo.lock().await.insert(gb_node);
        node_repo.lock().await.insert(onvif_node);

        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        let owner = service.assign(tenant, device, "gb28181", None).await?;
        assert_eq!(owner.owner_node_id, gb_node_id);
        Ok(())
    }

    #[tokio::test]
    async fn prefers_zone_then_falls_back() -> TestResult<()> {
        let (service, clock, node_repo, _owner_repo, id_gen) = setup();
        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;
        let zone_a = make_node(&id_gen, "zone-a", &["gb28181"], 0, 10, lease);
        let zone_b = make_node(&id_gen, "zone-b", &["gb28181"], 0, 10, lease);
        node_repo.lock().await.insert(zone_a.clone());
        node_repo.lock().await.insert(zone_b.clone());

        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        let owner = service
            .assign(tenant, device, "gb28181", Some("zone-b"))
            .await?;
        assert_eq!(owner.owner_node_id, zone_b.node_id);
        Ok(())
    }

    #[tokio::test]
    async fn enforces_rate_limit() -> TestResult<()> {
        let clock = Arc::new(InMemoryClock::new());
        let node_repo = Arc::new(AsyncMutex::new(FakeNodeRepository::new()));
        let owner_repo = Arc::new(AsyncMutex::new(FakeOwnerRepository::new(clock.clone())));
        let id_gen = InMemoryIdGenerator::new();
        let service = DeviceAssignmentService::new(
            node_repo.clone(),
            owner_repo.clone(),
            clock.clone(),
            DurationMs::from_millis(60_000),
            RateLimitConfig {
                global_per_second: 1,
                per_node_per_second: 1,
            },
        );

        let now = UtcTimestamp::parse_rfc3339("2026-01-01T00:00:00Z")?;
        clock.set_wall_ms(wall_ms(now));
        let lease = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or("lease overflow")?;
        let node = make_node(&id_gen, "zone-a", &["gb28181"], 0, 10, lease);
        node_repo.lock().await.insert(node);

        let tenant = id_gen.generate_tenant_id();
        let device1 = id_gen.generate_device_id();
        let _ = service.assign(tenant, device1, "gb28181", None).await?;

        let device2 = id_gen.generate_device_id();
        let result = service.assign(tenant, device2, "gb28181", None).await;
        assert!(matches!(result, Err(DeviceAssignmentError::RateLimited)));
        Ok(())
    }

    #[tokio::test]
    async fn no_available_node() -> TestResult<()> {
        let (service, _clock, _node_repo, _owner_repo, id_gen) = setup();
        let tenant = id_gen.generate_tenant_id();
        let device = id_gen.generate_device_id();
        let result = service.assign(tenant, device, "gb28181", None).await;
        assert!(matches!(
            result,
            Err(DeviceAssignmentError::NoAvailableNode)
        ));
        Ok(())
    }
}

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

/// Maximum number of nodes retained in the per-node assignment rate limiter.
const MAX_TRACKED_NODES: usize = 1_000_000;

/// Assigns devices to cluster nodes while preserving existing owners when
/// possible and respecting node health, zone, protocol support and load.
pub struct DeviceAssignmentService {
    node_repository: Arc<tokio::sync::Mutex<dyn NodeRepository>>,
    owner_repository: Arc<tokio::sync::Mutex<dyn OwnerRepository>>,
    clock: Arc<dyn Clock>,
    lease_duration: DurationMs,
    rate_limiter: Arc<Mutex<RateLimiter>>,
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
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(rate_limit))),
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
        if let Some(owner) = maybe_owner.clone()
            && owner.lease_until.is_none_or(|lease| lease > now)
            && let Some(node) = self.get_node(owner.owner_node_id).await?
            && is_owner_alive(&node, protocol, now)
        {
            return Ok(owner);
        }

        // Select the best candidate from the live cluster view.
        let candidate = self
            .select_node(tenant_id, device_id, protocol, preferred_zone, now)
            .await?;

        let now_mono = self.clock.now_monotonic();

        // Reserve an assignment slot atomically. The slot is committed only
        // after a successful ownership change; it rolls back automatically on
        // renewal, lost race, or error.
        let permit = {
            let mut limiter = self.rate_limiter.lock().map_err(|e| {
                StorageError::internal(format!("assignment rate limiter poisoned: {e}"))
            })?;
            if !limiter.reserve(now_mono, candidate.node_id) {
                return Err(DeviceAssignmentError::RateLimited);
            }
            RateLimitPermit::new(Arc::clone(&self.rate_limiter), candidate.node_id, now_mono)
        };

        // Acquire ownership on the selected node. If another node won the
        // race, return the owner it established.
        let mut owner_repo = self.owner_repository.lock().await;
        match owner_repo
            .acquire(tenant_id, device_id, candidate.node_id, now, lease_until)
            .await
        {
            Ok(owner) => {
                // Only consume the reserved slot when the owner actually
                // changed; renewals and lost races must not burn slots.
                let changed = maybe_owner.as_ref().is_none_or(|previous| {
                    previous.owner_node_id != owner.owner_node_id
                        || previous.owner_epoch != owner.owner_epoch
                });
                if changed {
                    permit.commit();
                }
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
    /// Maximum number of nodes to retain in the per-node rate limiter.
    pub max_tracked_nodes: usize,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            global_per_second: 1_000,
            per_node_per_second: 100,
            max_tracked_nodes: 4_096,
        }
    }
}

struct SlidingWindow {
    max_per_window: u32,
    window: DurationMs,
    events: VecDeque<DurationMs>,
    pending: u32,
    last_used: Option<DurationMs>,
}

impl SlidingWindow {
    fn new(max_per_window: u32, window: DurationMs) -> Self {
        Self {
            max_per_window,
            window,
            events: VecDeque::new(),
            pending: 0,
            last_used: None,
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

    /// Reserves one slot in the window if capacity is available, returning
    /// whether the reservation succeeded.
    fn reserve(&mut self, now: DurationMs) -> bool {
        self.prune(now);
        if self.events.len() + self.pending as usize >= self.max_per_window as usize {
            return false;
        }
        self.pending += 1;
        true
    }

    /// Converts a previously reserved slot into a committed event.
    fn commit(&mut self, now: DurationMs) {
        self.last_used = Some(now);
        self.pending = self.pending.saturating_sub(1);
        self.events.push_back(now);
    }

    /// Releases a previously reserved slot without recording an event.
    fn rollback(&mut self) {
        self.pending = self.pending.saturating_sub(1);
    }

    fn is_active(&mut self, now: DurationMs) -> bool {
        self.prune(now);
        !self.events.is_empty() || self.pending > 0
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
        let max_tracked_nodes = config.max_tracked_nodes.min(MAX_TRACKED_NODES);
        Self {
            global: SlidingWindow::new(config.global_per_second, window),
            per_node: HashMap::with_capacity(max_tracked_nodes),
            config: RateLimitConfig {
                max_tracked_nodes,
                ..config
            },
        }
    }

    /// Reserves one assignment slot atomically. Callers must either `commit`
    /// the reservation on success or `rollback` on failure/no-change.
    fn reserve(&mut self, now: DurationMs, node_id: NodeId) -> bool {
        if !self.global.reserve(now) {
            return false;
        }
        let window = self.per_node.entry(node_id).or_insert_with(|| {
            SlidingWindow::new(
                self.config.per_node_per_second,
                DurationMs::from_millis(1_000),
            )
        });
        if window.reserve(now) {
            true
        } else {
            self.global.rollback();
            false
        }
    }

    fn commit(&mut self, now: DurationMs, node_id: NodeId) {
        self.global.commit(now);
        if let Some(window) = self.per_node.get_mut(&node_id) {
            window.commit(now);
        }
        self.trim_per_node(now);
    }

    fn rollback(&mut self, node_id: NodeId) {
        self.global.rollback();
        if let Some(window) = self.per_node.get_mut(&node_id) {
            window.rollback();
        }
    }

    fn trim_per_node(&mut self, now: DurationMs) {
        if self.per_node.len() <= self.config.max_tracked_nodes {
            return;
        }

        // First drop windows whose events have all expired; they no longer
        // affect rate-limit accounting.
        self.per_node.retain(|_, w| w.is_active(now));

        // If still over capacity, evict the least-recently-used tracked node.
        if self.per_node.len() > self.config.max_tracked_nodes {
            let mut ordered: Vec<(NodeId, Option<DurationMs>)> = self
                .per_node
                .iter()
                .map(|(k, v)| (*k, v.last_used))
                .collect();
            ordered.sort_by_key(|a| a.1);
            let excess = self.per_node.len() - self.config.max_tracked_nodes;
            for (key, _) in ordered.into_iter().take(excess) {
                self.per_node.remove(&key);
            }
        }
    }
}

/// A reserved assignment rate-limit slot. Commits on success and rolls back
/// automatically if dropped without being committed.
struct RateLimitPermit {
    limiter: Arc<Mutex<RateLimiter>>,
    node_id: NodeId,
    now: DurationMs,
    committed: bool,
}

impl RateLimitPermit {
    fn new(limiter: Arc<Mutex<RateLimiter>>, node_id: NodeId, now: DurationMs) -> Self {
        Self {
            limiter,
            node_id,
            now,
            committed: false,
        }
    }

    fn commit(mut self) {
        let mut limiter = self.limiter.lock().unwrap_or_else(|e| e.into_inner());
        limiter.commit(self.now, self.node_id);
        self.committed = true;
    }
}

impl Drop for RateLimitPermit {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut limiter = self.limiter.lock().unwrap_or_else(|e| e.into_inner());
        limiter.rollback(self.node_id);
    }
}

/// Checks whether a node can accept a new device assignment.
fn is_eligible(node: &ClusterNode, protocol: &str, now: UtcTimestamp) -> bool {
    if !is_owner_alive(node, protocol, now) {
        return false;
    }
    if node.draining {
        return false;
    }
    if node.load.devices >= node.capacity.max_devices {
        return false;
    }
    true
}

/// Checks whether an existing owner's node is still alive for the protocol.
/// Unlike `is_eligible`, this ignores capacity and draining, which only affect
/// new assignments: an already-owned device stays with its node until the
/// owner lease expires and a graceful migration can occur.
fn is_owner_alive(node: &ClusterNode, protocol: &str, now: UtcTimestamp) -> bool {
    if node.lease_until <= now {
        return false;
    }
    if !protocol.is_empty() && !node.contract_versions.contains_key(protocol) {
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
    use cheetah_signal_types::{
        IdGenerator, ListCursor, NodeInstanceId, OwnerEpoch, Page, PageRequest,
    };
    use cheetah_storage_api::OwnedDevice;
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
            Err(StorageError::internal("test fake: not implemented"))
        }

        async fn heartbeat(
            &mut self,
            _node_id: NodeId,
            _instance_id: NodeInstanceId,
            _lease_until: UtcTimestamp,
            _updated_at: UtcTimestamp,
            _load: NodeLoad,
        ) -> Result<Option<ClusterNode>, StorageError> {
            Err(StorageError::internal("test fake: not implemented"))
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
            Err(StorageError::internal("test fake: not implemented"))
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
            Err(StorageError::internal("test fake: not implemented"))
        }

        async fn release(
            &mut self,
            _tenant_id: TenantId,
            _device_id: DeviceId,
            _node_id: NodeId,
            _epoch: OwnerEpoch,
        ) -> Result<(), StorageError> {
            Err(StorageError::internal("test fake: not implemented"))
        }

        async fn list_by_node(
            &self,
            node_id: NodeId,
            page: PageRequest,
        ) -> Result<Page<OwnedDevice>, StorageError> {
            let owners = self.owners();
            let mut matches: Vec<((TenantId, DeviceId), OwnerInfo)> = owners
                .iter()
                .filter(|(_, o)| o.owner_node_id == node_id)
                .map(|(k, o)| (*k, o.clone()))
                .collect();
            matches.sort_by_key(|a| a.0.1.as_uuid());

            let page_size = page.page_size_as_usize_clamped();
            let start = match &page.cursor {
                None => 0,
                Some(value) => {
                    let cursor = ListCursor::decode(value).map_err(|e| {
                        StorageError::invalid_argument(format!("invalid cursor: {e}"))
                    })?;
                    let (_, id) = cursor.parse().map_err(|e| {
                        StorageError::invalid_argument(format!("invalid cursor: {e}"))
                    })?;
                    matches
                        .iter()
                        .position(|((_, device_id), _)| device_id.as_uuid() > id)
                        .unwrap_or(matches.len())
                }
            };

            let end = (start + page_size + 1).min(matches.len());
            let has_more = end - start > page_size;
            let selected = &matches[start..end.min(start + page_size)];
            let next_cursor = if has_more {
                if let Some(last) = selected.last() {
                    Some(
                        ListCursor::new(UtcTimestamp::default(), last.0.1.as_uuid())
                            .map_err(|e| {
                                StorageError::internal(format!("failed to encode cursor: {e}"))
                            })?
                            .encode()
                            .map_err(|e| {
                                StorageError::internal(format!("failed to encode cursor: {e}"))
                            })?,
                    )
                } else {
                    None
                }
            } else {
                None
            };

            let items: Vec<OwnedDevice> = selected
                .iter()
                .map(|((tenant_id, device_id), owner)| OwnedDevice {
                    tenant_id: *tenant_id,
                    device_id: *device_id,
                    owner: owner.clone(),
                })
                .collect();

            let mut result = Page::new(items);
            if let Some(cursor) = next_cursor {
                result = result.with_next_cursor(cursor);
            }
            Ok(result)
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
        let owner_repo = Arc::new(AsyncMutex::new(FakeOwnerRepository::new()));
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
        let owner_repo = Arc::new(AsyncMutex::new(FakeOwnerRepository::new()));
        let id_gen = InMemoryIdGenerator::new();
        let service = DeviceAssignmentService::new(
            node_repo.clone(),
            owner_repo.clone(),
            clock.clone(),
            DurationMs::from_millis(60_000),
            RateLimitConfig {
                global_per_second: 1,
                per_node_per_second: 1,
                max_tracked_nodes: 16,
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

    #[test]
    fn rate_limiter_clamps_huge_tracked_nodes() {
        let config = RateLimitConfig {
            max_tracked_nodes: usize::MAX,
            ..RateLimitConfig::default()
        };
        let limiter = RateLimiter::new(config);
        assert_eq!(limiter.config.max_tracked_nodes, MAX_TRACKED_NODES);
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

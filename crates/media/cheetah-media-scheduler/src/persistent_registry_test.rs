//! Unit tests for `PersistentMediaNodeRegistry`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::model::{MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus};
use crate::persistent_registry::PersistentMediaNodeRegistry;
use crate::registry::MediaNodeRegistry;
use cheetah_signal_types::test_support::{FakeClock, FakeIdGenerator};
use cheetah_signal_types::{Clock, DurationMs, Event, NodeId, Page, PageRequest, UtcTimestamp};
use cheetah_storage_api::{MediaNodeRepository, StorageError};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct FakeRepo {
    node: Arc<Mutex<Option<MediaNode>>>,
    list_count: Arc<Mutex<u32>>,
    multi_page: Arc<Mutex<bool>>,
    fail_after_first_page: Arc<Mutex<bool>>,
}

impl FakeRepo {
    fn new() -> Self {
        Self {
            node: Arc::new(Mutex::new(None)),
            list_count: Arc::new(Mutex::new(0)),
            multi_page: Arc::new(Mutex::new(false)),
            fail_after_first_page: Arc::new(Mutex::new(false)),
        }
    }

    fn set_multi_page(&self, multi_page: bool) {
        *self.multi_page.lock().unwrap() = multi_page;
    }

    fn set_fail_after_first_page(&self, fail: bool) {
        *self.fail_after_first_page.lock().unwrap() = fail;
    }

    fn list_count(&self) -> u32 {
        *self.list_count.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl MediaNodeRepository for FakeRepo {
    async fn register(
        &mut self,
        mut node: MediaNode,
        _events: Vec<Event<cheetah_domain::DomainEvent>>,
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
        _events: Vec<Event<cheetah_domain::DomainEvent>>,
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
        let mut count = self.list_count.lock().unwrap();
        *count += 1;
        let fail = *self.fail_after_first_page.lock().unwrap();

        let node = self.node.lock().unwrap().clone();
        let Some(node) = node else {
            return Ok(Page::new(Vec::new()));
        };

        if let Some(lease) = node.lease_until {
            if now >= lease {
                return Ok(Page::new(Vec::new()));
            }
        } else {
            return Ok(Page::new(Vec::new()));
        }

        if fail && *count > 1 {
            return Err(StorageError::backend("list_alive failed"));
        }

        let multi_page = *self.multi_page.lock().unwrap();
        let mut page = Page::new(vec![node]);
        if multi_page && *count == 1 {
            page.next_cursor = Some("page-2".to_string());
        }
        Ok(page)
    }

    async fn set_draining(
        &mut self,
        _node_id: NodeId,
        _instance_id: String,
        _draining: bool,
        _updated_at: UtcTimestamp,
        _events: Vec<Event<cheetah_domain::DomainEvent>>,
    ) -> Result<Option<MediaNode>, StorageError> {
        Ok(None)
    }

    async fn deregister(
        &mut self,
        _node_id: NodeId,
        _instance_id: String,
        _updated_at: UtcTimestamp,
        _lease_until: Option<UtcTimestamp>,
        _events: Vec<Event<cheetah_domain::DomainEvent>>,
    ) -> Result<Option<MediaNode>, StorageError> {
        Ok(None)
    }
}

fn node_id() -> NodeId {
    NodeId::from_str("22222222-2222-2222-2222-222222222222").unwrap()
}

fn test_node(clock: &FakeClock) -> MediaNode {
    let now = clock.now_wall();
    let lease = now
        .checked_add(DurationMs::from_millis(60_000))
        .unwrap_or(now);
    MediaNode {
        node_id: node_id(),
        instance_id: "instance-1".to_string(),
        instance_epoch: 1,
        zone: "zone-a".to_string(),
        region: "region-1".to_string(),
        network_zones: vec!["zone-a".to_string()],
        labels: BTreeMap::new(),
        control_endpoint: "http://127.0.0.1:9000".to_string(),
        media_addresses: Vec::new(),
        capabilities: vec![MediaCapability {
            protocol: "gb28181".to_string(),
            operations: vec!["live".to_string()],
            constraints: BTreeMap::new(),
            version: 1,
            runtime_state: "active".to_string(),
        }],
        capacity: MediaNodeCapacity {
            max_sessions: 4,
            max_bandwidth_mbps: 1000,
            max_cpu_percent: 80,
        },
        load: 0,
        session_count: 0,
        health: MediaNodeHealth::Healthy,
        draining: false,
        status: NodeStatus::Active,
        last_heartbeat_at: Some(now),
        lease_until: Some(lease),
        generation: 1,
        contract_version: 1,
        revision: 1,
    }
}

fn registry(repo: FakeRepo) -> PersistentMediaNodeRegistry {
    PersistentMediaNodeRegistry::new(
        crate::config::MediaRegistryConfig::default(),
        Box::new(repo),
        Arc::new(FakeIdGenerator::new()),
        node_id(),
    )
}

#[tokio::test]
async fn list_active_loads_and_caches_nodes() {
    let clock = FakeClock::new();
    let mut repo = FakeRepo::new();
    repo.register(test_node(&clock), Vec::new()).await.unwrap();

    let reg = registry(repo.clone());
    let active = reg.list_active(&clock).await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].node_id, node_id());
    let initial_count = repo.list_count();
    assert!(initial_count >= 1);

    let active2 = reg.list_active(&clock).await;
    assert_eq!(active2.len(), 1);
    assert_eq!(repo.list_count(), initial_count); // cache hit, no repo call
}

#[tokio::test]
async fn list_active_does_not_cache_partial_results_on_repo_error() {
    let clock = FakeClock::new();
    let mut repo = FakeRepo::new();
    repo.register(test_node(&clock), Vec::new()).await.unwrap();
    repo.set_multi_page(true);
    repo.set_fail_after_first_page(true);

    let reg = registry(repo.clone());
    let active = reg.list_active(&clock).await;
    assert!(active.is_empty());
    assert_eq!(repo.list_count(), 2); // first page + failed second page

    repo.set_fail_after_first_page(false);
    let active = reg.list_active(&clock).await;
    assert_eq!(active.len(), 1);
    assert_eq!(repo.list_count(), 3); // retries because cache is still empty
}

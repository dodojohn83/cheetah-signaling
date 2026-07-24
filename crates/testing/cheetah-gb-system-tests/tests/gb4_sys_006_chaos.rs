//! GB4-SYS-006: deterministic cluster chaos / rolling-upgrade lifecycle test.
//!
//! Frame-level transport faults (drop/delay/reorder/duplicate/malformed/
//! sip_error) are covered deterministically by the GB4-TST-004 simulator DSL
//! (see `tools/gb28181-simulator/scenarios/chaos-cluster.toml` and its
//! `tests/chaos.rs`). This test covers the *lifecycle* faults that the
//! frame DSL cannot express, against a disposable PostgreSQL container:
//!
//! - node drain + owner migration via `DrainingMigrationService`;
//! - bounded drain (a per-drain device budget yields `truncated`), then
//!   convergence when the drain resumes;
//! - transaction timeout: an expired owner lease forces re-acquisition with a
//!   strictly higher owner epoch (stale epochs are fenced);
//! - service restart: reopening storage recovers the converged ownership.
//!
//! All time is driven by a fake clock; ids are deterministic; no RTP/RTCP/
//! PS/TS/ES payload is produced.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cheetah_cluster_ownership::{
    DeviceAssignmentService, DeviceProtocolLookup, DrainingMigrationService, RateLimitConfig,
    RollingUpgradeError,
};
use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, request_context};
use cheetah_domain::{ClusterNode, IdGenerator, NodeCapacity, NodeLoad, Tenant};
use cheetah_signal_application::{DeviceService, RegisterDeviceRequest};
use cheetah_signal_types::{Clock, DeviceId, DurationMs, NodeId, TenantId, UtcTimestamp};
use cheetah_storage_api::{OwnerRepository, Storage};
use cheetah_storage_postgres::{PostgresNodeRepository, PostgresOwnerRepository, PostgresStorage};
use testcontainers_modules::postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

async fn register_camera(
    service: &DeviceService,
    ctx: &cheetah_signal_types::RequestContext,
    uow: &mut dyn cheetah_domain::UnitOfWork,
    external_id: &str,
) -> DeviceId {
    service
        .register_or_update_device(
            ctx,
            uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: external_id.to_string(),
                authority: Some("chaos".to_string()),
                name: format!("camera-{external_id}"),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device
        .device_id
}

struct Gb28181Lookup;

#[async_trait::async_trait]
impl DeviceProtocolLookup for Gb28181Lookup {
    async fn protocol_for(
        &self,
        _tenant_id: TenantId,
        _device_id: DeviceId,
    ) -> Result<Option<String>, RollingUpgradeError> {
        Ok(Some("gb28181".to_string()))
    }
}

fn make_node(
    node_id: NodeId,
    instance: cheetah_signal_types::NodeInstanceId,
    lease_until: UtcTimestamp,
) -> ClusterNode {
    let mut node = ClusterNode::new(node_id, instance, "zone-a", "1.0.0", lease_until);
    node.updated_at = lease_until;
    node.started_at = lease_until;
    node.capacity = NodeCapacity { max_devices: 1000 };
    node.load = NodeLoad { devices: 0 };
    node.contract_versions = [("gb28181".to_string(), "1".to_string())]
        .into_iter()
        .collect();
    node
}

#[tokio::test(flavor = "multi_thread")]
async fn cluster_chaos_drain_migrate_timeout_and_restart() {
    let pg_container = postgres::Postgres::default().start().await.unwrap();
    let pg_host = pg_container.get_host().await.unwrap();
    let pg_port = pg_container.get_host_port_ipv4(5432).await.unwrap();
    let pg_url =
        format!("postgres://postgres:postgres@{pg_host}:{pg_port}/postgres?sslmode=disable");

    let storage = Arc::new(PostgresStorage::new(&pg_url).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock_impl = Arc::new(InMemoryClock::new());
    let clock: Arc<dyn Clock> = clock_impl.clone();
    let id_generator = Arc::new(InMemoryIdGenerator::new());

    let tenant = id_generator.generate_tenant_id();
    storage
        .tenant_repository()
        .save(&Tenant::new(tenant, "chaos-tenant", clock.now_wall()).unwrap())
        .await
        .unwrap();

    // Devices must exist before ownership rows can reference them.
    let device_service = DeviceService::new(clock.clone(), id_generator.clone());
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());
    let mut uow = storage.begin().await.unwrap();
    let device_1 = register_camera(&device_service, &ctx, &mut *uow, "34020000001320000001").await;
    let device_2 = register_camera(&device_service, &ctx, &mut *uow, "34020000001320000002").await;
    uow.commit().await.unwrap();
    let node_a = id_generator.generate_node_id();
    let node_b = id_generator.generate_node_id();
    let node_far_lease = clock
        .now_wall()
        .checked_add(DurationMs::from_seconds(3600))
        .unwrap();

    let node_repo: Arc<dyn cheetah_storage_api::NodeRepository> = Arc::new(
        PostgresNodeRepository::new(storage.read_pool().clone(), storage.write_pool().clone()),
    );
    let owner_repo: Arc<dyn OwnerRepository> = Arc::new(PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
    ));

    // Register node A only, so both devices deterministically land on it.
    node_repo
        .register(make_node(
            node_a,
            id_generator.generate_node_instance_id(),
            node_far_lease,
        ))
        .await
        .unwrap();

    // Short owner lease so a lease expiry (transaction timeout) is easy to force.
    let assignment = DeviceAssignmentService::new(
        node_repo.clone(),
        owner_repo.clone(),
        clock.clone(),
        DurationMs::from_millis(100),
        RateLimitConfig::default(),
    );

    let owner_1 = assignment
        .assign(tenant, device_1, "gb28181", None)
        .await
        .unwrap();
    let owner_2 = assignment
        .assign(tenant, device_2, "gb28181", None)
        .await
        .unwrap();
    assert_eq!(owner_1.owner_node_id, node_a);
    assert_eq!(owner_2.owner_node_id, node_a);

    // Bring node B online as the migration target.
    node_repo
        .register(make_node(
            node_b,
            id_generator.generate_node_instance_id(),
            node_far_lease,
        ))
        .await
        .unwrap();

    let migration = DrainingMigrationService::new(
        DeviceAssignmentService::new(
            node_repo.clone(),
            owner_repo.clone(),
            clock.clone(),
            DurationMs::from_millis(100),
            RateLimitConfig::default(),
        ),
        owner_repo.clone(),
        node_repo.clone(),
        clock.clone(),
        16,
        // Bounded: only one device per drain pass.
        1,
    );
    let lookup = Gb28181Lookup;

    // First drain pass migrates one device and reports truncation.
    let report_1 = migration.drain_node(node_a, &lookup).await.unwrap();
    assert_eq!(report_1.migrated, 1);
    assert!(report_1.truncated, "bounded drain must report truncation");

    // Resume the drain; it converges with no work truncated.
    let report_2 = migration.drain_node(node_a, &lookup).await.unwrap();
    assert_eq!(report_2.migrated, 1);
    assert!(!report_2.truncated, "resumed drain must converge");

    // Both devices are now owned by the surviving node, off the drained node.
    for device in [device_1, device_2] {
        let owner = owner_repo
            .get(tenant, device)
            .await
            .unwrap()
            .expect("owner after migration");
        assert_eq!(
            owner.owner_node_id, node_b,
            "device migrated off drained node"
        );
    }

    let migrated_epoch = owner_repo
        .get(tenant, device_1)
        .await
        .unwrap()
        .unwrap()
        .owner_epoch;

    // Transaction timeout: advance past the owner lease and re-assign. The
    // expired lease forces a fresh acquire with a strictly higher epoch.
    clock_impl.advance(DurationMs::from_millis(101));
    let reacquired = assignment
        .assign(tenant, device_1, "gb28181", None)
        .await
        .unwrap();
    assert!(
        reacquired.owner_epoch.0 > migrated_epoch.0,
        "expired lease must force a higher epoch"
    );

    // Service restart: drop every handle and reopen storage on the same
    // database. The converged ownership must survive the restart.
    drop(assignment);
    drop(migration);
    drop(node_repo);
    drop(owner_repo);
    Arc::try_unwrap(storage)
        .expect("no outstanding storage references")
        .close()
        .await
        .unwrap();

    let restarted = PostgresStorage::new(&pg_url).await.unwrap();
    let restarted_owner = PostgresOwnerRepository::new(
        restarted.read_pool().clone(),
        restarted.write_pool().clone(),
    );
    let recovered = restarted_owner
        .get(tenant, device_1)
        .await
        .unwrap()
        .expect("owner recovered after restart");
    assert_eq!(recovered.owner_epoch, reacquired.owner_epoch);
    restarted.close().await.unwrap();
}

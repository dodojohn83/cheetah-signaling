//! Takeover service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryIdGenerator, request_context as in_memory_request_context,
};
use cheetah_domain::{
    ClusterNode, CommandPayload, Device, DeviceKind, NodeCapacity, NodeLoad, OperationStatus,
    OwnerInfo, Protocol, PtzDirection,
};
use cheetah_signal_application::{
    OperationService, OwnerValidation, SubmitOperationRequest, TakeoverResult, TakeoverService,
};
use cheetah_signal_types::{
    Clock, DurationMs, IdGenerator, NodeInstanceId, OwnerEpoch, ProtocolIdentity, ResourceId,
    ResourceKind, ResourceRef,
};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

struct TestContext {
    storage: Arc<SqliteStorage>,
    clock: Arc<InMemoryClock>,
    id_generator: Arc<InMemoryIdGenerator>,
    tenant_id: cheetah_signal_types::TenantId,
    device_id: cheetah_signal_types::DeviceId,
    this_node: cheetah_signal_types::NodeId,
    old_node: cheetah_signal_types::NodeId,
    takeover_service: TakeoverService,
    operation_service: OperationService,
}

async fn setup(path: &std::path::Path) -> TestContext {
    let storage = Arc::new(SqliteStorage::new(path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<InMemoryClock> = Arc::new(InMemoryClock::new());
    let id_generator: Arc<InMemoryIdGenerator> = Arc::new(InMemoryIdGenerator::new());
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let this_node = id_generator.generate_node_id();
    let old_node = id_generator.generate_node_id();

    let now = clock.now_wall();
    let far_future = now
        .checked_add(DurationMs::from_millis(60_000_000))
        .unwrap();

    let mut this_cluster_node = ClusterNode::new(
        this_node,
        NodeInstanceId::from_uuid(id_generator.generate_node_id().as_uuid()),
        "zone-a",
        "0.1.0",
        now,
    );
    this_cluster_node.lease_until = far_future;
    this_cluster_node.capacity = NodeCapacity { max_devices: 100 };
    this_cluster_node.load = NodeLoad { devices: 0 };

    let mut old_cluster_node = ClusterNode::new(
        old_node,
        NodeInstanceId::from_uuid(id_generator.generate_node_id().as_uuid()),
        "zone-a",
        "0.1.0",
        now,
    );
    old_cluster_node.lease_until = now;
    old_cluster_node.capacity = NodeCapacity { max_devices: 100 };
    old_cluster_node.load = NodeLoad { devices: 1 };

    {
        let node_repo = storage.node_repository();
        node_repo.register(this_cluster_node).await.unwrap();
        node_repo.register(old_cluster_node).await.unwrap();
    }

    let takeover_service = TakeoverService::new(
        storage.clone(),
        clock.clone(),
        id_generator.clone(),
        this_node,
        DurationMs::from_millis(60_000),
    );
    let operation_service = OperationService::new(clock.clone(), id_generator.clone());

    TestContext {
        storage,
        clock,
        id_generator,
        tenant_id,
        device_id,
        this_node,
        old_node,
        takeover_service,
        operation_service,
    }
}

fn remove_db(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

async fn register_device(ctx: &TestContext) {
    let (device, event) = Device::new(
        ctx.clock.as_ref(),
        ctx.tenant_id,
        ctx.device_id,
        Protocol::Gb28181,
        ProtocolIdentity::new("ext-takeover-1").unwrap(),
        "authority",
        "camera-01",
        DeviceKind::Camera,
        Vec::new(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();

    let mut uow = ctx.storage.begin().await.unwrap();
    uow.device_repository().save(&device).await.unwrap();
    uow.commit().await.unwrap();
    let _ = event;
}

async fn submit_operation(ctx: &TestContext) -> cheetah_signal_types::OperationId {
    let context = in_memory_request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);
    let target = ResourceRef {
        tenant_id: ctx.tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(ctx.device_id),
    };
    let request = SubmitOperationRequest {
        device_id: ctx.device_id,
        target,
        payload: CommandPayload::Ptz {
            channel_id: ctx.id_generator.generate_channel_id(),
            direction: PtzDirection::Right,
            speed: 0.5,
        },
        idempotency_key: "ptz-takeover".to_string(),
        deadline: None,
        expected_owner_epoch: OwnerEpoch(0),
    };

    let mut uow = ctx.storage.begin().await.unwrap();
    let operation = ctx
        .operation_service
        .submit_operation(&context, uow.as_mut(), request)
        .await
        .unwrap();
    operation.operation_id
}

async fn set_old_owner(ctx: &TestContext) {
    let repo = ctx.storage.owner_repository();
    let now = ctx.clock.now_wall();
    let lease_until = now
        .checked_add(DurationMs::from_millis(60_000_000))
        .unwrap();
    repo.set(
        ctx.tenant_id,
        ctx.device_id,
        OwnerInfo {
            owner_node_id: ctx.old_node,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(lease_until),
        },
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn takeover_service_takes_over_dead_node_and_fails_stale_operation() {
    let file_id = InMemoryIdGenerator::new()
        .generate_message_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_takeover_test_{file_id}.db"));
    remove_db(&path);
    let ctx = setup(&path).await;

    register_device(&ctx).await;
    let operation_id = submit_operation(&ctx).await;
    set_old_owner(&ctx).await;

    // Old owner node lease is already expired (`lease_until = now`), so takeover
    // should win for this node.
    let context = in_memory_request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);
    let result = ctx
        .takeover_service
        .takeover(&context, ctx.tenant_id, ctx.device_id)
        .await
        .unwrap();

    let (owner, recovered) = match result {
        TakeoverResult::Acquired { owner, recovered } => (owner, recovered),
        other => panic!("expected Acquired, got {other:?}"),
    };
    assert_eq!(owner.owner_node_id, ctx.this_node);
    assert_eq!(owner.owner_epoch, OwnerEpoch(2));
    assert_eq!(recovered, 1);

    let mut uow = ctx.storage.begin().await.unwrap();
    let operation = uow
        .operation_repository()
        .get(ctx.tenant_id, operation_id)
        .await
        .unwrap()
        .expect("operation should exist");
    assert_eq!(operation.status(), OperationStatus::Failed);
    assert!(operation.error().is_some());
    assert_eq!(operation.error().unwrap().code(), "owner_lost");
    uow.rollback().await.unwrap();
    remove_db(&path);
}

#[tokio::test(flavor = "multi_thread")]
async fn takeover_service_validates_local_owner() {
    let file_id = InMemoryIdGenerator::new()
        .generate_message_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_takeover_validate_{file_id}.db"));
    remove_db(&path);
    let ctx = setup(&path).await;

    register_device(&ctx).await;
    set_old_owner(&ctx).await;

    let old_owner = OwnerInfo {
        owner_node_id: ctx.old_node,
        owner_epoch: OwnerEpoch(1),
        lease_until: None,
    };
    assert_eq!(
        ctx.takeover_service
            .validate_local_owner(ctx.tenant_id, ctx.device_id, &old_owner)
            .await
            .unwrap(),
        OwnerValidation::Current
    );

    let context = in_memory_request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);
    let _ = ctx
        .takeover_service
        .takeover(&context, ctx.tenant_id, ctx.device_id)
        .await
        .unwrap();

    assert_eq!(
        ctx.takeover_service
            .validate_local_owner(ctx.tenant_id, ctx.device_id, &old_owner)
            .await
            .unwrap(),
        OwnerValidation::Fenced
    );

    remove_db(&path);
}

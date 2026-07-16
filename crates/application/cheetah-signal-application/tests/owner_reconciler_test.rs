//! Owner reconciler integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
use cheetah_domain::{Device, DeviceKind, IdGenerator, OwnerInfo, Protocol};
use cheetah_signal_application::{LocalDeviceSession, OwnerReconciler};
use cheetah_signal_types::{
    Clock, DeviceId, DurationMs, NodeId, OwnerEpoch, ProtocolIdentity, TenantId, UtcTimestamp,
};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

struct TestContext {
    storage: Arc<SqliteStorage>,
    clock: Arc<InMemoryClock>,
    id_generator: Arc<InMemoryIdGenerator>,
    tenant_id: TenantId,
    this_node: NodeId,
    other_node: NodeId,
}

async fn setup(path: &std::path::Path) -> TestContext {
    let storage = Arc::new(SqliteStorage::new(path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<InMemoryClock> = Arc::new(InMemoryClock::new());
    let id_generator: Arc<InMemoryIdGenerator> = Arc::new(InMemoryIdGenerator::new());
    let tenant_id = id_generator.generate_tenant_id();
    let this_node = id_generator.generate_node_id();
    let other_node = id_generator.generate_node_id();

    TestContext {
        storage,
        clock,
        id_generator,
        tenant_id,
        this_node,
        other_node,
    }
}

fn remove_db(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

async fn register_device(ctx: &TestContext, device_id: DeviceId, external_id: &str) -> Device {
    let (device, _event) = Device::new(
        ctx.clock.as_ref(),
        ctx.tenant_id,
        device_id,
        Protocol::Gb28181,
        ProtocolIdentity::new(external_id).unwrap(),
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
    device
}

fn far_future(ctx: &TestContext) -> UtcTimestamp {
    ctx.clock
        .now_wall()
        .checked_add(DurationMs::from_millis(60_000_000))
        .unwrap()
}

fn far_past(ctx: &TestContext) -> UtcTimestamp {
    ctx.clock
        .now_wall()
        .checked_sub(DurationMs::from_millis(60_000_000))
        .unwrap()
}

async fn set_owner(ctx: &TestContext, device_id: DeviceId, owner: OwnerInfo) {
    let mut repo = ctx.storage.owner_repository();
    repo.set(ctx.tenant_id, device_id, owner).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_reconciler_matches_current_owner() {
    let file_id = InMemoryIdGenerator::new()
        .generate_operation_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_owner_reconciler_match_{file_id}.db"));
    remove_db(&path);
    let ctx = setup(&path).await;

    let device_id = ctx.id_generator.generate_device_id();
    register_device(&ctx, device_id, "ext-owner-match").await;

    let owner = OwnerInfo {
        owner_node_id: ctx.this_node,
        owner_epoch: OwnerEpoch(2),
        lease_until: Some(far_future(&ctx)),
    };
    set_owner(&ctx, device_id, owner.clone()).await;

    let reconciler = OwnerReconciler::new(
        ctx.this_node,
        ctx.storage.owner_repository().into(),
        ctx.clock.clone(),
        10,
    );

    let session = LocalDeviceSession {
        tenant_id: ctx.tenant_id,
        device_id,
        local_owner: Some(owner),
    };
    let report = reconciler.reconcile(&[session]).await.unwrap();

    assert_eq!(report.scanned, 1);
    assert_eq!(report.matched, 1);
    assert_eq!(report.orphaned, 0);
    assert!(report.orphaned_sessions.is_empty());

    remove_db(&path);
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_reconciler_flags_orphaned_sessions() {
    let file_id = InMemoryIdGenerator::new()
        .generate_operation_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_owner_reconciler_orphan_{file_id}.db"));
    remove_db(&path);
    let ctx = setup(&path).await;

    let owned_by_other = ctx.id_generator.generate_device_id();
    let expired = ctx.id_generator.generate_device_id();
    let missing = ctx.id_generator.generate_device_id();

    register_device(&ctx, owned_by_other, "ext-owner-other").await;
    register_device(&ctx, expired, "ext-owner-expired").await;
    register_device(&ctx, missing, "ext-owner-missing").await;

    set_owner(
        &ctx,
        owned_by_other,
        OwnerInfo {
            owner_node_id: ctx.other_node,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(far_future(&ctx)),
        },
    )
    .await;
    set_owner(
        &ctx,
        expired,
        OwnerInfo {
            owner_node_id: ctx.this_node,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(far_past(&ctx)),
        },
    )
    .await;

    let reconciler = OwnerReconciler::new(
        ctx.this_node,
        ctx.storage.owner_repository().into(),
        ctx.clock.clone(),
        10,
    );

    let sessions = [
        LocalDeviceSession {
            tenant_id: ctx.tenant_id,
            device_id: owned_by_other,
            local_owner: None,
        },
        LocalDeviceSession {
            tenant_id: ctx.tenant_id,
            device_id: expired,
            local_owner: None,
        },
        LocalDeviceSession {
            tenant_id: ctx.tenant_id,
            device_id: missing,
            local_owner: None,
        },
    ];
    let report = reconciler.reconcile(&sessions).await.unwrap();

    assert_eq!(report.scanned, 3);
    assert_eq!(report.matched, 0);
    assert_eq!(report.orphaned, 3);
    assert_eq!(report.orphaned_sessions.len(), 3);

    remove_db(&path);
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_reconciler_detects_epoch_mismatch() {
    let file_id = InMemoryIdGenerator::new()
        .generate_operation_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_owner_reconciler_epoch_{file_id}.db"));
    remove_db(&path);
    let ctx = setup(&path).await;

    let device_id = ctx.id_generator.generate_device_id();
    register_device(&ctx, device_id, "ext-owner-epoch").await;

    set_owner(
        &ctx,
        device_id,
        OwnerInfo {
            owner_node_id: ctx.this_node,
            owner_epoch: OwnerEpoch(3),
            lease_until: Some(far_future(&ctx)),
        },
    )
    .await;

    let reconciler = OwnerReconciler::new(
        ctx.this_node,
        ctx.storage.owner_repository().into(),
        ctx.clock.clone(),
        10,
    );

    let session = LocalDeviceSession {
        tenant_id: ctx.tenant_id,
        device_id,
        local_owner: Some(OwnerInfo {
            owner_node_id: ctx.this_node,
            owner_epoch: OwnerEpoch(2),
            lease_until: Some(far_future(&ctx)),
        }),
    };
    let report = reconciler
        .reconcile(std::slice::from_ref(&session))
        .await
        .unwrap();

    assert_eq!(report.scanned, 1);
    assert_eq!(report.matched, 0);
    assert_eq!(report.orphaned, 1);
    assert_eq!(report.orphaned_sessions, vec![session]);

    remove_db(&path);
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_reconciler_respects_batch_size() {
    let file_id = InMemoryIdGenerator::new()
        .generate_operation_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_owner_reconciler_batch_{file_id}.db"));
    remove_db(&path);
    let ctx = setup(&path).await;

    let device_id = ctx.id_generator.generate_device_id();
    register_device(&ctx, device_id, "ext-owner-batch").await;

    set_owner(
        &ctx,
        device_id,
        OwnerInfo {
            owner_node_id: ctx.this_node,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(far_future(&ctx)),
        },
    )
    .await;

    let reconciler = OwnerReconciler::new(
        ctx.this_node,
        ctx.storage.owner_repository().into(),
        ctx.clock.clone(),
        2,
    );

    let sessions = [
        LocalDeviceSession {
            tenant_id: ctx.tenant_id,
            device_id,
            local_owner: None,
        },
        LocalDeviceSession {
            tenant_id: ctx.tenant_id,
            device_id,
            local_owner: None,
        },
        LocalDeviceSession {
            tenant_id: ctx.tenant_id,
            device_id,
            local_owner: None,
        },
    ];
    let first = reconciler.reconcile(&sessions).await.unwrap();
    assert_eq!(first.scanned, 2);

    remove_db(&path);
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_reconciler_find_missing_paginates() {
    let file_id = InMemoryIdGenerator::new()
        .generate_operation_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_owner_reconciler_missing_{file_id}.db"));
    remove_db(&path);
    let ctx = setup(&path).await;

    let device_a = ctx.id_generator.generate_device_id();
    let device_b = ctx.id_generator.generate_device_id();
    register_device(&ctx, device_a, "ext-missing-a").await;
    register_device(&ctx, device_b, "ext-missing-b").await;

    set_owner(
        &ctx,
        device_a,
        OwnerInfo {
            owner_node_id: ctx.this_node,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(far_future(&ctx)),
        },
    )
    .await;
    set_owner(
        &ctx,
        device_b,
        OwnerInfo {
            owner_node_id: ctx.this_node,
            owner_epoch: OwnerEpoch(1),
            lease_until: Some(far_future(&ctx)),
        },
    )
    .await;

    let reconciler = OwnerReconciler::new(
        ctx.this_node,
        ctx.storage.owner_repository().into(),
        ctx.clock.clone(),
        1,
    );

    let local_sessions = [LocalDeviceSession {
        tenant_id: ctx.tenant_id,
        device_id: device_a,
        local_owner: None,
    }];

    let mut cursor = None;
    let mut total_db_scanned = 0u64;
    let mut total_missing = 0u64;
    let mut missing_devices = Vec::new();
    for _ in 0..10 {
        let report = reconciler
            .find_missing(&local_sessions, cursor)
            .await
            .unwrap();
        total_db_scanned += report.db_scanned;
        total_missing += report.missing;
        missing_devices.extend(report.missing_sessions.iter().map(|s| s.device_id));
        cursor = report.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    assert_eq!(total_db_scanned, 2);
    assert_eq!(total_missing, 1);
    assert_eq!(missing_devices, vec![device_b]);

    remove_db(&path);
}

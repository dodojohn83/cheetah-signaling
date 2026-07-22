//! Bounded lifecycle and crash-recovery system scenario (GB4-OPS-004).
//!
//! Uses a real SQLite backend, an in-process command/event bus and a fake
//! media node to verify:
//!
//! 1. **Startup order** — dependencies are assembled in the mandated order
//!    (`schema → bus → repository → ownership → media → services`) and the
//!    node reaches a ready state that can dispatch a command.
//! 2. **Graceful shutdown / drain** — pending outbox work is drained (relayed)
//!    before the node stops accepting new work.
//! 3. **Crash recovery** — after a simulated crash a new owner acquires the
//!    lease with an incremented epoch, commands stamped with the old owner
//!    epoch are rejected before any side effect, and outbox events committed
//!    before the crash are replayed exactly once.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use cheetah_cluster_ownership::{CachingDeviceOwnerResolver, OwnerLeaseService};
use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, request_context};
use cheetah_domain::{
    CommandPayload, DeviceOwnerResolver, IdGenerator, OperationStatus, PtzDirection,
};
use cheetah_message_api::bus::{RawCommandBus, RawEventBus};
use cheetah_message_api::mapper::decode_command;
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_application::{
    ChannelDescriptor, CommandDispatcher, DeviceService, MarkDeviceOnlineRequest, OperationService,
    OutboxRelay, RegisterDeviceRequest, ReplaceChannelCatalogRequest, SubmitOperationRequest,
};
use cheetah_signal_types::{Clock, DurationMs, OwnerEpoch, ResourceId, ResourceKind, ResourceRef};
use cheetah_storage_api::{OwnerRepository, Storage};
use cheetah_storage_sqlite::{SqliteOwnerRepository, SqliteStorage};

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_startup_graceful_shutdown_and_crash_recovery() {
    let file_id = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let base = std::env::temp_dir().join(format!("cheetah_lifecycle_recovery_{file_id}"));
    let db_path = base.with_extension("db");
    let wal_path = base.with_extension("db-wal");
    let shm_path = base.with_extension("db-shm");

    // ---- Startup order: schema -> bus -> repository -> ownership -> media -> services.
    let storage = Arc::new(SqliteStorage::new(&db_path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let bus = Arc::new(InProcessMessageBus::new(16, 16));

    let owner_repo_shared = Arc::new(tokio::sync::Mutex::new(SqliteOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
    ))) as Arc<tokio::sync::Mutex<dyn OwnerRepository>>;
    let owner_repo_for_resolver: Arc<dyn OwnerRepository> = Arc::new(SqliteOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
    ));
    // Short TTL so the resolver observes epoch changes across a crash.
    let resolver: Arc<dyn DeviceOwnerResolver> = Arc::new(CachingDeviceOwnerResolver::new(
        owner_repo_for_resolver.clone(),
        clock.clone(),
        DurationMs::from_millis(1),
        128,
    ));

    let node_a = id_generator.generate_node_id();
    let node_b = id_generator.generate_node_id();
    let lease_a = OwnerLeaseService::new(
        owner_repo_shared.clone(),
        clock.clone(),
        node_a,
        DurationMs::from_millis(60_000),
    );

    let device_service = DeviceService::new(clock.clone(), id_gen_dyn.clone());
    let operation_service = OperationService::new(clock.clone(), id_gen_dyn.clone());
    let dispatcher = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        resolver.clone(),
        bus.clone(),
    );

    let tenant = id_generator.generate_tenant_id();
    let channel_id = id_generator.generate_channel_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());

    // Register + activate a device with a channel.
    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-lifecycle".to_string(),
                authority: Some("auth".to_string()),
                name: "lifecycle-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;
    let device_id = device.device_id;
    device_service
        .mark_device_online(
            &ctx,
            &mut *uow,
            device_id,
            MarkDeviceOnlineRequest { reason: None },
        )
        .await
        .unwrap();
    device_service
        .replace_channel_catalog(
            &ctx,
            &mut *uow,
            device_id,
            ReplaceChannelCatalogRequest {
                channels: vec![ChannelDescriptor {
                    id: Some(channel_id.to_string()),
                    name: "ch".to_string(),
                    kind: "video".to_string(),
                    enabled: true,
                    status: Some("online".to_string()),
                    stream_profiles: Vec::new(),
                    ptz_capabilities: None,
                    metadata: None,
                }],
            },
        )
        .await
        .unwrap();
    uow.commit().await.unwrap();

    // Ownership acquired by node A: epoch 1 (ready).
    let owner_a = lease_a.acquire(tenant, device_id).await.unwrap();
    assert_eq!(
        owner_a.owner_epoch,
        OwnerEpoch(1),
        "first acquire is epoch 1"
    );
    assert_eq!(owner_a.owner_node_id, node_a);

    let target = ResourceRef {
        tenant_id: tenant,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    };

    // ---- Healthy dispatch under the current owner epoch.
    let mut uow = storage.begin().await.unwrap();
    let op1 = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: target.clone(),
                payload: CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "op-1".to_string(),
                deadline: None,
                expected_owner_epoch: owner_a.owner_epoch,
            },
        )
        .await
        .unwrap();
    uow.commit().await.unwrap();

    let mut sub = RawCommandBus::subscribe(bus.as_ref(), "", "")
        .await
        .unwrap();
    let mut uow = storage.begin().await.unwrap();
    let dispatched = dispatcher
        .dispatch(&ctx, &mut *uow, tenant, op1.operation_id)
        .await
        .unwrap();
    assert_eq!(dispatched.status, OperationStatus::Running);
    let delivery = tokio::time::timeout(Duration::from_secs(1), sub.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command delivered under valid epoch");
    assert_eq!(
        decode_command(&delivery.envelope).unwrap().operation_id(),
        op1.operation_id
    );

    // ---- Graceful shutdown: drain the outbox before stopping.
    let relay = OutboxRelay::new(storage.clone(), bus.clone(), clock.clone(), 10);
    let drained = relay.run_once().await.unwrap();
    assert!(
        drained >= 1,
        "graceful drain must relay pending outbox events"
    );
    let mut uow = storage.begin().await.unwrap();
    let pending = uow.outbox().pending(clock.now_wall(), 10).await.unwrap();
    uow.commit().await.unwrap();
    assert!(pending.is_empty(), "outbox fully drained before shutdown");

    // ---- Simulate work committed just before a crash: op2 is dispatched (its
    // Start event is written to the outbox in the same transaction) but the
    // relay never runs, modelling a crash after commit but before publication.
    let mut uow = storage.begin().await.unwrap();
    let op2 = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: target.clone(),
                payload: CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "op-2".to_string(),
                deadline: None,
                expected_owner_epoch: owner_a.owner_epoch,
            },
        )
        .await
        .unwrap();
    uow.commit().await.unwrap();
    let mut uow = storage.begin().await.unwrap();
    dispatcher
        .dispatch(&ctx, &mut *uow, tenant, op2.operation_id)
        .await
        .unwrap();
    uow.commit().await.unwrap();
    let mut uow = storage.begin().await.unwrap();
    let pending_before_crash = uow.outbox().pending(clock.now_wall(), 10).await.unwrap();
    uow.commit().await.unwrap();
    assert!(
        !pending_before_crash.is_empty(),
        "op2 leaves an unpublished outbox event at crash time"
    );

    // ================= CRASH =================
    // Drop node A's in-memory services and bus subscription. Storage (the
    // durable state) survives.
    drop(sub);

    // ---- Recovery: a fresh node B restarts against the same storage.
    let bus_b = Arc::new(InProcessMessageBus::new(16, 16));
    let lease_b = OwnerLeaseService::new(
        owner_repo_shared.clone(),
        clock.clone(),
        node_b,
        DurationMs::from_millis(60_000),
    );
    // Node B restarts with a fresh owner cache (its in-memory state was lost in
    // the crash), so it re-reads the durable owner epoch from storage.
    let resolver_b: Arc<dyn DeviceOwnerResolver> = Arc::new(CachingDeviceOwnerResolver::new(
        owner_repo_for_resolver.clone(),
        clock.clone(),
        DurationMs::from_millis(1),
        128,
    ));
    let dispatcher_b = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        resolver_b.clone(),
        bus_b.clone(),
    );

    // New owner acquires the lease: epoch is atomically incremented.
    let owner_b = lease_b.acquire(tenant, device_id).await.unwrap();
    assert_eq!(
        owner_b.owner_epoch,
        OwnerEpoch(2),
        "re-acquire by a new node increments the owner epoch"
    );
    assert_eq!(owner_b.owner_node_id, node_b);

    // ---- Old owner epoch rejected: a command stamped with epoch 1 must not
    // reach the bus after the epoch has advanced to 2.
    let mut uow = storage.begin().await.unwrap();
    let stale_op = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: target.clone(),
                payload: CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "op-stale".to_string(),
                deadline: None,
                expected_owner_epoch: owner_a.owner_epoch,
            },
        )
        .await
        .unwrap();
    uow.commit().await.unwrap();

    let mut sub_b = RawCommandBus::subscribe(bus_b.as_ref(), "", "")
        .await
        .unwrap();
    let mut uow = storage.begin().await.unwrap();
    let stale_dispatched = dispatcher_b
        .dispatch(&ctx, &mut *uow, tenant, stale_op.operation_id)
        .await
        .unwrap();
    uow.commit().await.unwrap();
    assert_eq!(
        stale_dispatched.status,
        OperationStatus::Failed,
        "stale owner epoch command must fail before dispatch"
    );
    let stale_delivery = tokio::time::timeout(Duration::from_millis(200), sub_b.next()).await;
    assert!(
        stale_delivery.is_err(),
        "no command may be delivered for a stale owner epoch"
    );

    // ---- Outbox replay: the pre-crash pending event (plus the stale-op events)
    // are replayed by the recovered relay onto the event bus. Subscribe before
    // relaying since the in-process event bus is broadcast.
    let mut event_sub_b = RawEventBus::subscribe(bus_b.as_ref(), "", "")
        .await
        .unwrap();
    let relay_b = OutboxRelay::new(storage.clone(), bus_b.clone(), clock.clone(), 10);
    let replayed = relay_b.run_once().await.unwrap();
    assert!(
        replayed >= 1,
        "recovered relay must replay outbox events committed before the crash"
    );
    let first_replayed = tokio::time::timeout(Duration::from_secs(1), event_sub_b.next())
        .await
        .unwrap()
        .unwrap()
        .expect("replayed event delivered after recovery");
    assert!(!first_replayed.message_id.is_empty());

    let mut uow = storage.begin().await.unwrap();
    let pending_after_replay = uow.outbox().pending(clock.now_wall(), 10).await.unwrap();
    uow.commit().await.unwrap();
    assert!(
        pending_after_replay.is_empty(),
        "outbox must be empty after recovery replay"
    );

    drop(uow);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);
}

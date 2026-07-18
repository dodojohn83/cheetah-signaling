//! PERF-002: cluster scale performance scenario.
//!
//! Uses a real PostgreSQL backend and an in-memory command bus to measure
//! device registration, heartbeat and command dispatch throughput under
//! concurrent load. The scenario is marked `#[ignore]` and is intended to be
//! run manually against a local Docker or testcontainers environment.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator, request_context,
};
use cheetah_domain::{CommandPayload, OperationStatus, OwnerInfo, PtzDirection};
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_application::{
    ChannelDescriptor, CommandDispatcher, DeviceService, MarkDeviceOnlineRequest, OperationService,
    RegisterDeviceRequest, ReplaceChannelCatalogRequest, SubmitOperationRequest,
};
use cheetah_signal_types::{Clock, IdGenerator, OwnerEpoch, ResourceId, ResourceKind, ResourceRef};
use cheetah_storage_api::Storage;
use cheetah_storage_postgres::PostgresStorage;
use testcontainers_modules::postgres;
use testcontainers_modules::testcontainers::{ContainerAsync, runners::AsyncRunner};

mod perf_common;

const DEVICE_COUNT: usize = 100;
const HEARTBEAT_ITERATIONS: usize = 100;
const COMMAND_ITERATIONS: usize = 100;
// Heartbeat/command dispatch are measured on the same device; use concurrency 1
// to avoid optimistic-lock conflicts on the device aggregate. Registration uses
// unique external IDs and can run concurrently.
const REGISTRATION_CONCURRENCY: usize = 2;
const HEARTBEAT_COMMAND_CONCURRENCY: usize = 1;

async fn wait_for_postgres_ready(
    url: &str,
    timeout: Duration,
) -> Result<PostgresStorage, cheetah_storage_api::StorageError> {
    let started = std::time::Instant::now();
    loop {
        match PostgresStorage::new(url).await {
            Ok(storage) => return Ok(storage),
            Err(e) if started.elapsed() >= timeout => return Err(e),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

async fn setup_cluster() -> (
    ContainerAsync<postgres::Postgres>,
    Arc<PostgresStorage>,
    Arc<dyn Clock>,
    Arc<InMemoryIdGenerator>,
    Arc<dyn IdGenerator>,
    DeviceService,
    OperationService,
    CommandDispatcher,
    Arc<InMemoryDeviceOwnerResolver>,
) {
    let pg_container = postgres::Postgres::default().start().await.unwrap();
    let pg_port = pg_container.get_host_port_ipv4(5432).await.unwrap();
    let pg_host = pg_container.get_host().await.unwrap().to_string();
    let pg_url =
        format!("postgres://postgres:postgres@{pg_host}:{pg_port}/postgres?sslmode=disable");

    let storage = Arc::new(
        wait_for_postgres_ready(&pg_url, Duration::from_secs(30))
            .await
            .unwrap(),
    );
    storage.migration().run().await.unwrap();

    // Warm the connection pool before concurrent scenarios so the first
    // acquisitions don't pay connection-establishment cost under the timeout.
    for _ in 0..5 {
        let mut uow = storage.begin().await.unwrap();
        uow.commit().await.unwrap();
    }

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let owner_resolver = Arc::new(InMemoryDeviceOwnerResolver::new());
    let command_bus = Arc::new(InProcessMessageBus::new(1024, 1024));
    let device_service = DeviceService::new(clock.clone(), id_gen_dyn.clone());
    let operation_service = OperationService::new(clock.clone(), id_gen_dyn.clone());
    let command_dispatcher = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        command_bus.clone(),
    );

    // Pre-seed the node as owner for all generated devices.
    (
        pg_container,
        storage,
        clock,
        id_generator,
        id_gen_dyn,
        device_service,
        operation_service,
        command_dispatcher,
        owner_resolver,
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual performance test"]
async fn perf_cluster_device_registration() {
    let (
        _pg_container,
        storage,
        clock,
        id_generator,
        _id_gen_dyn,
        device_service,
        _operation_service,
        _dispatcher,
        _owner_resolver,
    ) = setup_cluster().await;

    let tenant = id_generator.generate_tenant_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());

    let summary = perf_common::measure_concurrent(
        "cluster_device_registration",
        REGISTRATION_CONCURRENCY,
        DEVICE_COUNT / REGISTRATION_CONCURRENCY,
        move || {
            let ctx = ctx.clone();
            let device_service = device_service.clone();
            let storage = storage.clone();
            let id_generator = id_generator.clone();
            async move {
                let mut uow = storage.begin().await.unwrap();
                device_service
                    .register_or_update_device(
                        &ctx,
                        &mut *uow,
                        RegisterDeviceRequest {
                            protocol: "gb28181".to_string(),
                            external_id: format!("perf-{}", id_generator.generate_device_id()),
                            authority: Some("auth".to_string()),
                            name: "perf-camera".to_string(),
                            kind: "camera".to_string(),
                            capabilities: None,
                            metadata: None,
                        },
                    )
                    .await
                    .unwrap();
            }
        },
    )
    .await;

    summary.print();
    assert!(
        summary.p95_ns < 50_000_000,
        "p95 registration latency < 50ms"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual performance test"]
async fn perf_cluster_heartbeat_and_command() {
    let (
        _pg_container,
        storage,
        clock,
        id_generator,
        _id_gen_dyn,
        device_service,
        operation_service,
        command_dispatcher,
        owner_resolver,
    ) = setup_cluster().await;

    let tenant = id_generator.generate_tenant_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());
    let source_node_id = id_generator.generate_node_id();

    // Register a single device with one channel that will be used for both
    // heartbeats and command dispatch.
    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "heartbeat-cmd-target".to_string(),
                authority: Some("auth".to_string()),
                name: "cluster-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;
    let channel_id = id_generator.generate_channel_id();
    device_service
        .replace_channel_catalog(
            &ctx,
            &mut *uow,
            device.device_id,
            ReplaceChannelCatalogRequest {
                channels: vec![ChannelDescriptor {
                    id: Some(channel_id.to_string()),
                    name: "ch-1".to_string(),
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

    owner_resolver.set_owner(
        tenant,
        device.device_id,
        OwnerInfo {
            owner_node_id: source_node_id,
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

    let hb_ctx = ctx.clone();
    let hb_storage = storage.clone();
    let hb_device_service = device_service.clone();
    let hb_summary = perf_common::measure_concurrent(
        "cluster_heartbeat",
        HEARTBEAT_COMMAND_CONCURRENCY,
        HEARTBEAT_ITERATIONS / HEARTBEAT_COMMAND_CONCURRENCY,
        move || {
            let ctx = hb_ctx.clone();
            let device_service = hb_device_service.clone();
            let storage = hb_storage.clone();
            let device_id = device.device_id;
            async move {
                let mut uow = storage.begin().await.unwrap();
                device_service
                    .mark_device_online(
                        &ctx,
                        &mut *uow,
                        device_id,
                        MarkDeviceOnlineRequest { reason: None },
                    )
                    .await
                    .unwrap();
            }
        },
    )
    .await;
    hb_summary.print();
    assert!(
        hb_summary.p95_ns < 20_000_000,
        "p95 heartbeat latency < 20ms"
    );

    let cmd_ctx = ctx.clone();
    let cmd_storage = storage.clone();
    let cmd_operation_service = operation_service.clone();
    let cmd_command_dispatcher = command_dispatcher.clone();
    let cmd_id_generator = id_generator.clone();
    let cmd_summary = perf_common::measure_concurrent(
        "cluster_command_dispatch",
        HEARTBEAT_COMMAND_CONCURRENCY,
        COMMAND_ITERATIONS / HEARTBEAT_COMMAND_CONCURRENCY,
        move || {
            let ctx = cmd_ctx.clone();
            let operation_service = cmd_operation_service.clone();
            let command_dispatcher = cmd_command_dispatcher.clone();
            let storage = cmd_storage.clone();
            let id_generator = cmd_id_generator.clone();
            let device_id = device.device_id;
            async move {
                let mut uow = storage.begin().await.unwrap();
                let target = ResourceRef {
                    tenant_id: ctx.tenant_id,
                    kind: ResourceKind::Device,
                    id: ResourceId::Device(device_id),
                };
                let ptz = operation_service
                    .submit_operation(
                        &ctx,
                        &mut *uow,
                        SubmitOperationRequest {
                            device_id,
                            target,
                            payload: CommandPayload::Ptz {
                                channel_id,
                                direction: PtzDirection::Stop,
                                speed: 0.0,
                            },
                            idempotency_key: format!(
                                "ptz-{}",
                                id_generator.generate_operation_id()
                            ),
                            deadline: None,
                            expected_owner_epoch: OwnerEpoch::default(),
                        },
                    )
                    .await
                    .unwrap();
                let dispatched = command_dispatcher
                    .dispatch(&ctx, &mut *uow, ctx.tenant_id, ptz.operation_id)
                    .await
                    .unwrap();
                assert!(
                    dispatched.status == OperationStatus::Running,
                    "dispatch must succeed"
                );
            }
        },
    )
    .await;
    cmd_summary.print();
    assert!(
        cmd_summary.p95_ns < 30_000_000,
        "p95 command dispatch latency < 30ms"
    );
}

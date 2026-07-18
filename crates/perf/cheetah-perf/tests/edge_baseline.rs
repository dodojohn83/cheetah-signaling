//! PERF-001: edge baseline performance scenario.
//!
//! Runs on a single node with an on-disk SQLite database and in-memory ports.
//! Scenarios are marked `#[ignore]` so they do not run in normal CI.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::OwnerInfo;
use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator, InMemoryMediaPort,
    request_context,
};
use cheetah_signal_application::{
    ChannelDescriptor, DeviceService, MarkDeviceOnlineRequest, MediaService, OperationService,
    RegisterDeviceRequest, ReplaceChannelCatalogRequest, StartLiveRequest, StopLiveRequest,
};
use cheetah_signal_types::OwnerEpoch;
use cheetah_signal_types::{Clock, IdGenerator};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

mod perf_common;

const REGISTRATION_ITERATIONS: usize = 1000;
const HEARTBEAT_ITERATIONS: usize = 1000;
const MEDIA_SESSIONS: usize = 100;

async fn setup_edge() -> (
    tempfile::TempDir,
    Arc<SqliteStorage>,
    Arc<dyn Clock>,
    Arc<InMemoryIdGenerator>,
    Arc<dyn IdGenerator>,
    DeviceService,
    OperationService,
    MediaService,
    cheetah_signal_types::NodeId,
    Arc<InMemoryDeviceOwnerResolver>,
) {
    let tempdir = tempfile::tempdir().unwrap();
    let db_path = tempdir.path().join("perf.db");

    let storage = Arc::new(SqliteStorage::new(&db_path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let owner_resolver = Arc::new(InMemoryDeviceOwnerResolver::new());
    let media_port: Arc<dyn cheetah_domain::MediaPort> =
        Arc::new(InMemoryMediaPort::new(id_gen_dyn.clone()));
    let source_node_id = id_generator.generate_node_id();

    let device_service = DeviceService::new(clock.clone(), id_gen_dyn.clone());
    let operation_service = OperationService::new(clock.clone(), id_gen_dyn.clone());
    let media_service = MediaService::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        media_port,
        source_node_id,
    );

    (
        tempdir,
        storage,
        clock,
        id_generator,
        id_gen_dyn,
        device_service,
        operation_service,
        media_service,
        source_node_id,
        owner_resolver,
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual performance test"]
async fn perf_edge_device_registration() {
    let (
        _tempdir,
        storage,
        clock,
        id_generator,
        _id_gen_dyn,
        device_service,
        _operation_service,
        _media,
        _source_node,
        _owner_resolver,
    ) = setup_edge().await;

    let tenant = id_generator.generate_tenant_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());

    // Pre-seed the database with a device registration so migrations/connection
    // warm-up is not counted in the measured loop.
    let mut uow = storage.begin().await.unwrap();
    device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "warmup".to_string(),
                authority: Some("auth".to_string()),
                name: "warmup".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap();

    let summary = perf_common::measure("edge_device_registration", REGISTRATION_ITERATIONS, || {
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
    })
    .await;

    summary.print();
    assert!(summary.p95_ns < 5_000_000, "p95 registration latency < 5ms");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual performance test"]
async fn perf_edge_heartbeat() {
    let (
        _tempdir,
        storage,
        clock,
        id_generator,
        _id_gen_dyn,
        device_service,
        _operation_service,
        _media,
        _source_node,
        _owner_resolver,
    ) = setup_edge().await;

    let tenant = id_generator.generate_tenant_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());

    // Register the heartbeat target once.
    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "heartbeat-target".to_string(),
                authority: Some("auth".to_string()),
                name: "heartbeat-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;

    let summary = perf_common::measure("edge_heartbeat", HEARTBEAT_ITERATIONS, || {
        let ctx = ctx.clone();
        let device_service = device_service.clone();
        let storage = storage.clone();
        async move {
            let mut uow = storage.begin().await.unwrap();
            device_service
                .mark_device_online(
                    &ctx,
                    &mut *uow,
                    device.device_id,
                    MarkDeviceOnlineRequest { reason: None },
                )
                .await
                .unwrap();
        }
    })
    .await;

    summary.print();
    assert!(summary.p95_ns < 2_000_000, "p95 heartbeat latency < 2ms");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual performance test"]
async fn perf_edge_media_session_control() {
    let (
        _tempdir,
        storage,
        clock,
        id_generator,
        _id_gen_dyn,
        device_service,
        _operation_service,
        media_service,
        _source_node,
        owner_resolver,
    ) = setup_edge().await;

    let tenant = id_generator.generate_tenant_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());

    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "media-target".to_string(),
                authority: Some("auth".to_string()),
                name: "media-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;

    owner_resolver.set_owner(
        tenant,
        device.device_id,
        OwnerInfo {
            owner_node_id: _source_node,
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

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

    device_service
        .mark_device_online(
            &ctx,
            &mut *uow,
            device.device_id,
            MarkDeviceOnlineRequest { reason: None },
        )
        .await
        .unwrap();

    let summary = perf_common::measure("edge_media_session_start_stop", MEDIA_SESSIONS, || {
        let ctx = ctx.clone();
        let media_service = media_service.clone();
        let storage = storage.clone();
        let id_generator = id_generator.clone();
        let device_id = device.device_id;
        async move {
            let mut uow = storage.begin().await.unwrap();
            let session = media_service
                .start_live(
                    &ctx,
                    &mut *uow,
                    StartLiveRequest {
                        device_id: device_id.to_string(),
                        channel_id: channel_id.to_string(),
                        idempotency_key: format!("live-{}", id_generator.generate_operation_id()),
                        deadline: None,
                    },
                )
                .await
                .unwrap();
            media_service
                .stop_live(
                    &ctx,
                    &mut *uow,
                    StopLiveRequest {
                        media_session_id: session.media_session_id.to_string(),
                        idempotency_key: format!("stop-{}", id_generator.generate_operation_id()),
                    },
                )
                .await
                .unwrap();
        }
    })
    .await;

    summary.print();
    assert!(
        summary.p95_ns < 10_000_000,
        "p95 media start+stop latency < 10ms"
    );
}

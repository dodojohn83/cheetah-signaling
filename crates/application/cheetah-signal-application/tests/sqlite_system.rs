//! Single-node SQLite system scenario.
//!
//! Covers GB/ONVIF device onboarding, Operation/Command dispatch,
//! start/stop live media, restart recovery, and multi-tenant external-id
//! isolation using a real SQLite storage backend.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator, InMemoryMediaPort,
    request_context,
};
use cheetah_domain::{
    ChannelStatus, CommandPayload, DeviceLifecycle, MediaSessionState, OperationStatus, OwnerInfo,
    Protocol, PtzDirection,
};
use cheetah_message_api::bus::RawCommandBus;
use cheetah_message_api::mapper::decode_command;
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_application::{
    ChannelDescriptor, CommandDispatcher, DeviceService, MarkDeviceOnlineRequest, MediaService,
    OperationService, RegisterDeviceRequest, ReplaceChannelCatalogRequest, StartLiveRequest,
    StopLiveRequest, SubmitOperationRequest,
};
use cheetah_signal_types::{Clock, IdGenerator};
use cheetah_signal_types::{OwnerEpoch, ProtocolIdentity, ResourceId, ResourceKind, ResourceRef};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

#[tokio::test(flavor = "multi_thread")]
async fn single_node_sqlite_device_operation_and_media_lifecycle() {
    let file_id = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let base = std::env::temp_dir().join(format!("cheetah_system_sqlite_{file_id}"));
    let db_path = base.with_extension("db");
    let wal_path = base.with_extension("db-wal");
    let shm_path = base.with_extension("db-shm");

    let storage = Arc::new(SqliteStorage::new(&db_path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let owner_resolver: Arc<InMemoryDeviceOwnerResolver> =
        Arc::new(InMemoryDeviceOwnerResolver::new());
    let command_bus = Arc::new(InProcessMessageBus::new(16, 16));
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
    let command_dispatcher = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        command_bus.clone(),
    );

    let tenant_a = id_generator.generate_tenant_id();
    let tenant_b = id_generator.generate_tenant_id();
    let channel_a = id_generator.generate_channel_id();
    let channel_b = id_generator.generate_channel_id();

    let mut uow = storage.begin().await.unwrap();
    let ctx_a = request_context(tenant_a, id_generator.as_ref(), clock.as_ref());
    let ctx_b = request_context(tenant_b, id_generator.as_ref(), clock.as_ref());

    let gb = device_service
        .register_or_update_device(
            &ctx_a,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-shared".to_string(),
                authority: Some("auth-a".to_string()),
                name: "gb-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;

    let onvif = device_service
        .register_or_update_device(
            &ctx_b,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "onvif".to_string(),
                external_id: "ext-shared".to_string(),
                authority: Some("auth-b".to_string()),
                name: "onvif-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;

    assert_ne!(
        gb.device_id, onvif.device_id,
        "same external id must be isolated by tenant"
    );

    device_service
        .mark_device_online(
            &ctx_a,
            &mut *uow,
            gb.device_id,
            MarkDeviceOnlineRequest { reason: None },
        )
        .await
        .unwrap();
    device_service
        .mark_device_online(
            &ctx_b,
            &mut *uow,
            onvif.device_id,
            MarkDeviceOnlineRequest { reason: None },
        )
        .await
        .unwrap();

    device_service
        .replace_channel_catalog(
            &ctx_a,
            &mut *uow,
            gb.device_id,
            ReplaceChannelCatalogRequest {
                channels: vec![ChannelDescriptor {
                    id: Some(channel_a.to_string()),
                    name: "ch-a".to_string(),
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
        .replace_channel_catalog(
            &ctx_b,
            &mut *uow,
            onvif.device_id,
            ReplaceChannelCatalogRequest {
                channels: vec![ChannelDescriptor {
                    id: Some(channel_b.to_string()),
                    name: "ch-b".to_string(),
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
        tenant_a,
        gb.device_id,
        OwnerInfo {
            owner_node_id: source_node_id,
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

    let mut uow = storage.begin().await.unwrap();
    let session = media_service
        .start_live(
            &ctx_a,
            &mut *uow,
            StartLiveRequest {
                device_id: gb.device_id.to_string(),
                channel_id: channel_a.to_string(),
                idempotency_key: "live-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.state, MediaSessionState::Inviting);
    let operation_id = session.operation_id;

    let operation = operation_service
        .get_operation(&mut *uow, tenant_a, operation_id)
        .await
        .unwrap();
    assert_eq!(operation.status, OperationStatus::Running);

    uow.commit().await.unwrap();

    // Dispatch a PTZ command for the same device.
    let mut uow = storage.begin().await.unwrap();
    let ptz_target = ResourceRef {
        tenant_id: tenant_a,
        kind: ResourceKind::Device,
        id: ResourceId::Device(gb.device_id),
    };
    let ptz = operation_service
        .submit_operation(
            &ctx_a,
            &mut *uow,
            SubmitOperationRequest {
                device_id: gb.device_id,
                target: ptz_target,
                payload: CommandPayload::Ptz {
                    channel_id: channel_a,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-1".to_string(),
                deadline: None,
                expected_owner_epoch: OwnerEpoch::default(),
            },
        )
        .await
        .unwrap();

    let mut sub = command_bus
        .subscribe("", "")
        .await
        .expect("in-process command subscription");
    let mut uow = storage.begin().await.unwrap();
    let dispatched = command_dispatcher
        .dispatch(&ctx_a, &mut *uow, tenant_a, ptz.operation_id)
        .await
        .unwrap();
    assert_eq!(dispatched.status, OperationStatus::Running);

    let delivery = tokio::time::timeout(Duration::from_secs(1), sub.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command must be delivered");
    let decoded_command = decode_command(&delivery.envelope).unwrap();
    assert_eq!(decoded_command.operation_id(), ptz.operation_id);

    // Stop the live session and verify terminal state.
    let mut uow = storage.begin().await.unwrap();
    let stopped = media_service
        .stop_live(
            &ctx_a,
            &mut *uow,
            StopLiveRequest {
                media_session_id: session.media_session_id.to_string(),
                idempotency_key: "stop-1".to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(stopped.state, MediaSessionState::Stopped);
    let stop_scope = cheetah_domain::IdempotencyScope::new(
        tenant_a,
        "test",
        cheetah_signal_types::ResourceRef {
            tenant_id: tenant_a,
            kind: cheetah_signal_types::ResourceKind::MediaSession,
            id: cheetah_signal_types::ResourceId::MediaSession(stopped.media_session_id),
        },
        "stop-1",
    )
    .unwrap();
    let stop_operation = uow
        .operation_repository()
        .get_by_idempotency(&stop_scope)
        .await
        .unwrap()
        .expect("stop operation must exist");
    assert!(stop_operation.status().is_terminal());
    uow.commit().await.unwrap();

    // Simulate restart: new services, same storage, verify persisted state.
    let mut uow = storage.begin().await.unwrap();
    let recovered_gb = uow
        .device_repository()
        .get(tenant_a, gb.device_id)
        .await
        .unwrap()
        .expect("gb device must persist");
    assert_eq!(recovered_gb.lifecycle(), DeviceLifecycle::Active);

    let recovered_onvif = uow
        .device_repository()
        .get(tenant_b, onvif.device_id)
        .await
        .unwrap()
        .expect("onvif device must persist");
    assert_eq!(recovered_onvif.lifecycle(), DeviceLifecycle::Active);

    let recovered_session = uow
        .media_session_repository()
        .get(tenant_a, session.media_session_id)
        .await
        .unwrap()
        .expect("media session must persist");
    assert_eq!(recovered_session.state(), MediaSessionState::Stopped);

    let channel = uow
        .channel_repository()
        .get(tenant_a, gb.device_id, channel_a)
        .await
        .unwrap()
        .expect("channel must persist");
    assert_eq!(channel.status(), ChannelStatus::Online);

    let a_by_external = uow
        .device_repository()
        .get_by_external_id(
            tenant_a,
            Protocol::Gb28181,
            ProtocolIdentity::new("ext-shared").unwrap(),
        )
        .await
        .unwrap()
        .expect("tenant a device by external id");
    assert_eq!(a_by_external.device_id(), gb.device_id);

    let b_by_external = uow
        .device_repository()
        .get_by_external_id(
            tenant_b,
            Protocol::Onvif,
            ProtocolIdentity::new("ext-shared").unwrap(),
        )
        .await
        .unwrap()
        .expect("tenant b device by external id");
    assert_eq!(b_by_external.device_id(), onvif.device_id);

    // Drop the unit of work to release the SQLite transaction/connection.
    drop(uow);

    // Clean up temporary database files.
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);
}

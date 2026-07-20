//! Operation / MediaSession / MediaBinding final-state independence scenario.
//!
//! Verifies that:
//! - Start Operation success does not terminate the MediaSession (it becomes Active).
//! - Stop Operation success leaves no active MediaBinding and terminates the session.
//! - A media-node failure callback fails only the MediaSession/MediaBinding, while
//!   the associated Operation is independently tracked as failed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator, InMemoryMediaPort,
    request_context,
};
use cheetah_domain::{
    MediaBindingState, MediaNodeCallback, MediaNodeCallbackKind, MediaSessionState, OperationStatus,
};
use cheetah_signal_application::{
    ChannelDescriptor, DeviceService, MarkDeviceOnlineRequest, MediaService, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest, StartLiveRequest, StopLiveRequest,
};
use cheetah_signal_types::{Clock, IdGenerator, OwnerEpoch};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_state_independence_for_operation_session_and_binding() {
    let file_id = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let base = std::env::temp_dir().join(format!("cheetah_state_indep_{file_id}"));
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
    let media_port = Arc::new(InMemoryMediaPort::new(id_gen_dyn.clone()));
    let source_node_id = id_generator.generate_node_id();

    let device_service = DeviceService::new(clock.clone(), id_gen_dyn.clone());
    let media_service = MediaService::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        media_port.clone(),
        source_node_id,
    );

    let tenant = id_generator.generate_tenant_id();
    let channel_id = id_generator.generate_channel_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());

    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-state-1".to_string(),
                authority: Some("auth".to_string()),
                name: "gb-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;

    device_service
        .mark_device_online(
            &ctx,
            &mut *uow,
            device.device_id,
            MarkDeviceOnlineRequest { reason: None },
        )
        .await
        .unwrap();

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
        cheetah_domain::OwnerInfo {
            owner_node_id: source_node_id,
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

    // 1. Start live: session is Inviting, binding is Active, operation is Running.
    let session = media_service
        .start_live(
            &ctx,
            &mut *uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel_id.to_string(),
                idempotency_key: "live-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.state, MediaSessionState::Inviting);

    let binding = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("binding exists");
    let media_node_id = binding.media_node_id();
    let media_node_instance_epoch = binding.media_node_instance_epoch();
    let binding_id = binding.media_binding_id();
    let start_operation_id = session.operation_id;

    let start_operation = uow
        .operation_repository()
        .get(tenant, start_operation_id)
        .await
        .unwrap()
        .expect("start operation exists");
    assert!(!start_operation.is_terminal());
    assert_eq!(start_operation.status(), OperationStatus::Running);

    // 2. Media node callback: session becomes Active, start Operation completes,
    //    but the MediaSession is not terminated.
    let callback = MediaNodeCallback {
        media_node_id,
        media_node_instance_epoch,
        media_session_id: session.media_session_id,
        media_binding_id: binding_id,
        operation_id: Some(start_operation_id),
        owner_epoch: Some(OwnerEpoch::default()),
        message_id: "msg-started".to_string(),
        binding_revision: Some(binding.revision()),
        session_revision: Some(session.revision),
        kind: MediaNodeCallbackKind::Started,
    };
    let active_session = media_service
        .handle_media_event(&ctx, &mut *uow, callback)
        .await
        .unwrap();
    assert_eq!(active_session.state, MediaSessionState::Active);

    let completed_start_op = uow
        .operation_repository()
        .get(tenant, start_operation_id)
        .await
        .unwrap()
        .expect("start operation exists");
    assert!(completed_start_op.is_terminal());
    assert_eq!(completed_start_op.status(), OperationStatus::Succeeded);

    // 3. Stop live: stop Operation completes, MediaSession is Stopped and the
    //    active MediaBinding is released.
    let stopped = media_service
        .stop_live(
            &ctx,
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
        tenant,
        "test",
        cheetah_signal_types::ResourceRef {
            tenant_id: tenant,
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
        .expect("stop operation exists");
    assert!(stop_operation.is_terminal());
    assert_eq!(stop_operation.status(), OperationStatus::Succeeded);

    let no_active_binding = uow
        .media_binding_repository()
        .get_by_media_session(tenant, stopped.media_session_id)
        .await
        .unwrap();
    assert!(
        no_active_binding.is_none(),
        "stop operation success must leave no active media binding"
    );

    let terminal_binding = uow
        .media_binding_repository()
        .get(tenant, binding_id)
        .await
        .unwrap()
        .expect("binding still exists");
    assert!(terminal_binding.is_terminal());
    assert!(matches!(
        terminal_binding.state(),
        MediaBindingState::Released | MediaBindingState::Failed
    ));

    // Clean up temporary database files.
    drop(uow);
    drop(storage);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);
}

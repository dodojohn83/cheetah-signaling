//! Media-node failure, orphan detection and callback ordering system scenario.
//!
//! Uses a real SQLite storage backend and the in-memory media port to verify
//! `MediaService::reconcile` converges, fails and cleans up media sessions and
//! bindings in response to media-node reports.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator, InMemoryMediaPort,
    request_context,
};
use cheetah_domain::{MediaBindingState, MediaNodeSessionRef, MediaSessionState};
use cheetah_signal_application::{
    ChannelDescriptor, DeviceService, MarkDeviceOnlineRequest, MediaService, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest, StartLiveRequest,
};
use cheetah_signal_types::{Clock, IdGenerator, MediaNodeInstanceEpoch, OwnerEpoch};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_media_reconcile_active_missing_and_orphan_sessions() {
    let file_id = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let base = std::env::temp_dir().join(format!("cheetah_media_reconcile_{file_id}"));
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
                external_id: "ext-cam-1".to_string(),
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

    // Start live creates an Inviting session and Active binding.
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

    uow = storage.begin().await.unwrap();
    let binding = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("binding exists");
    let binding_id = binding.media_binding_id();
    let media_node_id = binding.media_node_id();
    let media_node_instance_epoch = binding.media_node_instance_epoch();

    // 1. Media node reports the session -> reconcile converges to Active.
    media_port.set_node_sessions(
        tenant,
        media_node_id,
        vec![MediaNodeSessionRef {
            media_session_id: session.media_session_id,
            device_id: Some(device.device_id),
            channel_id: Some(channel_id),
            media_node_instance_epoch,
        }],
    );

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();
    assert_eq!(report.nodes_scanned, 1);
    assert_eq!(report.sessions_found, 1);
    assert_eq!(report.missing_failed, 0);
    assert_eq!(report.orphans_detected, 0);

    let active_session = uow
        .media_session_repository()
        .get(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("session exists");
    assert_eq!(active_session.state(), MediaSessionState::Active);
    let active_binding = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("binding exists");
    assert_eq!(active_binding.state(), MediaBindingState::Active);

    // 2. Media node instance epoch mismatch -> session fails (out-of-order/stale callback).
    media_port.set_node_sessions(
        tenant,
        media_node_id,
        vec![MediaNodeSessionRef {
            media_session_id: session.media_session_id,
            device_id: Some(device.device_id),
            channel_id: Some(channel_id),
            media_node_instance_epoch: MediaNodeInstanceEpoch(media_node_instance_epoch.0 + 1),
        }],
    );

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();
    assert_eq!(report.missing_failed, 1);

    let failed_session = uow
        .media_session_repository()
        .get(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("session exists");
    assert!(failed_session.is_terminal());
    let failed_binding = uow
        .media_binding_repository()
        .get(tenant, binding_id)
        .await
        .unwrap()
        .expect("binding exists");
    assert!(failed_binding.is_terminal());

    // The first media node no longer reports the failed session.
    media_port.set_node_sessions(tenant, media_node_id, vec![]);

    // 3. Start a second live session and report an orphan alongside it.
    let session2 = media_service
        .start_live(
            &ctx,
            &mut *uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel_id.to_string(),
                idempotency_key: "live-2".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    uow = storage.begin().await.unwrap();

    let binding2 = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session2.media_session_id)
        .await
        .unwrap()
        .expect("binding2 exists");
    let binding2_id = binding2.media_binding_id();
    let node2 = binding2.media_node_id();
    let epoch2 = binding2.media_node_instance_epoch();
    let orphan_session_id = id_generator.generate_media_session_id();
    media_port.set_node_sessions(
        tenant,
        node2,
        vec![
            MediaNodeSessionRef {
                media_session_id: session2.media_session_id,
                device_id: Some(device.device_id),
                channel_id: Some(channel_id),
                media_node_instance_epoch: epoch2,
            },
            MediaNodeSessionRef {
                media_session_id: orphan_session_id,
                device_id: Some(device.device_id),
                channel_id: Some(channel_id),
                media_node_instance_epoch: epoch2,
            },
        ],
    );

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();
    assert_eq!(report.orphans_detected, 1);
    assert_eq!(report.sessions_found, 2);

    let session2_active = uow
        .media_session_repository()
        .get(tenant, session2.media_session_id)
        .await
        .unwrap()
        .expect("session2 exists");
    assert_eq!(session2_active.state(), MediaSessionState::Active);

    // 4. Clear the node report -> missing active session is migrated to a new
    // media node rather than failed; the old binding is terminated.
    media_port.set_node_sessions(tenant, node2, vec![]);
    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();
    assert_eq!(report.missing_failed, 0);
    assert_eq!(report.migrations_succeeded, 1);

    let migrated_session = uow
        .media_session_repository()
        .get(tenant, session2.media_session_id)
        .await
        .unwrap()
        .expect("session2 still exists");
    assert_eq!(migrated_session.state(), MediaSessionState::Active);
    let migrated_binding = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session2.media_session_id)
        .await
        .unwrap()
        .expect("migrated binding exists");
    assert_eq!(migrated_binding.state(), MediaBindingState::Active);
    let old_binding = uow
        .media_binding_repository()
        .get(tenant, binding2_id)
        .await
        .unwrap()
        .expect("binding2 still exists");
    assert!(old_binding.is_terminal());

    // Clean up temporary database files.
    drop(uow);
    drop(storage);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_media_reconcile_releases_stopped_binding() {
    let file_id = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let base = std::env::temp_dir().join(format!("cheetah_media_reconcile_release_{file_id}"));
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
                external_id: "ext-release-1".to_string(),
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

    // Start live creates an Inviting session and Active binding.
    let session = media_service
        .start_live(
            &ctx,
            &mut *uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel_id.to_string(),
                idempotency_key: "release-live-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.state, MediaSessionState::Inviting);

    // Simulate an external stop that leaves the session terminal but the
    // binding still active. Reconciliation must fully release the binding.
    let mut media_session = uow
        .media_session_repository()
        .get(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("session exists");
    let binding_id = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("binding exists")
        .media_binding_id();
    media_session.stop(&*clock).unwrap();
    uow.media_session_repository()
        .save(&media_session)
        .await
        .unwrap();

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    assert_eq!(report.missing_released, 1);

    let media_session = uow
        .media_session_repository()
        .get(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("session exists");
    let binding = uow
        .media_binding_repository()
        .get(tenant, binding_id)
        .await
        .unwrap()
        .expect("binding exists");
    assert_eq!(media_session.state(), MediaSessionState::Stopped);
    assert_eq!(binding.state(), MediaBindingState::Released);

    // Clean up temporary database files.
    drop(uow);
    drop(storage);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);
}

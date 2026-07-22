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
use cheetah_domain::{
    MediaBindingState, MediaNode, MediaNodeHealth, MediaNodeSessionRef, MediaPort,
    MediaSessionState, NodeStatus,
};
use cheetah_signal_application::{
    ChannelDescriptor, DeviceService, MarkDeviceOnlineRequest, MediaService, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest, StartLiveRequest,
};
use cheetah_signal_types::{
    Clock, DurationMs, IdGenerator, MediaNodeInstanceEpoch, OwnerEpoch, PageRequest,
};
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

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_media_reconcile_needs_verification_and_recovery() {
    let file_id = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let base = std::env::temp_dir().join(format!("cheetah_media_reconcile_verify_{file_id}"));
    let db_path = base.with_extension("db");
    let wal_path = base.with_extension("db-wal");
    let shm_path = base.with_extension("db-shm");

    let storage = Arc::new(SqliteStorage::new(&db_path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<InMemoryClock> = Arc::new(InMemoryClock::new());
    let clock_dyn: Arc<dyn Clock> = clock.clone();
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let owner_resolver: Arc<InMemoryDeviceOwnerResolver> =
        Arc::new(InMemoryDeviceOwnerResolver::new());
    let media_port = Arc::new(InMemoryMediaPort::new(id_gen_dyn.clone()));
    let source_node_id = id_generator.generate_node_id();

    let device_service = DeviceService::new(clock_dyn.clone(), id_gen_dyn.clone());
    let mut media_service = MediaService::new(
        clock_dyn.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        media_port.clone(),
        source_node_id,
    );
    media_service.set_needs_verification_grace_ms(10_000);

    let tenant = id_generator.generate_tenant_id();
    let channel_id = id_generator.generate_channel_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock_dyn.as_ref());

    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-verify-1".to_string(),
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

    let session = media_service
        .start_live(
            &ctx,
            &mut *uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel_id.to_string(),
                idempotency_key: "verify-live-1".to_string(),
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
    assert_eq!(report.needs_verification, 0);

    let active_binding = uow
        .media_binding_repository()
        .get(tenant, binding_id)
        .await
        .unwrap()
        .expect("binding exists");
    assert_eq!(active_binding.state(), MediaBindingState::Active);

    // 2. Node drops out of the active list but is still known to the registry as
    // unhealthy with a valid lease. The binding must move to NeedsVerification and
    // the session must stay Active.
    media_port.set_node_sessions(tenant, media_node_id, vec![]);
    let lease_until = clock_dyn
        .now_wall()
        .checked_add(DurationMs::from_millis(60_000))
        .expect("valid timestamp");
    let unhealthy_node = MediaNode {
        node_id: media_node_id,
        status: NodeStatus::Active,
        health: MediaNodeHealth::Unhealthy,
        lease_until: Some(lease_until),
        ..Default::default()
    };
    media_port.set_node(tenant, unhealthy_node);

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();
    assert_eq!(report.needs_verification, 1);
    assert_eq!(report.migrations_succeeded, 0);
    assert_eq!(report.missing_failed, 0);

    let verifying_binding = uow
        .media_binding_repository()
        .get(tenant, binding_id)
        .await
        .unwrap()
        .expect("binding exists");
    assert_eq!(
        verifying_binding.state(),
        MediaBindingState::NeedsVerification
    );

    let verifying_session = uow
        .media_session_repository()
        .get(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("session exists");
    assert_eq!(verifying_session.state(), MediaSessionState::Active);

    // 3. Node recovers and reports the session again. converge_active must drive
    // the binding back to Active without recreating it.
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
    media_port.remove_node(tenant, media_node_id);

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();
    assert_eq!(report.nodes_scanned, 1);
    assert_eq!(report.needs_verification, 0);

    let recovered_binding = uow
        .media_binding_repository()
        .get(tenant, binding_id)
        .await
        .unwrap()
        .expect("binding exists");
    assert_eq!(recovered_binding.state(), MediaBindingState::Active);

    // 4. Node becomes unhealthy again and stays beyond the grace window. The
    // reconciler must escalate and migrate the session to a new node.
    media_port.set_node_sessions(tenant, media_node_id, vec![]);
    let unhealthy_node = MediaNode {
        node_id: media_node_id,
        status: NodeStatus::Active,
        health: MediaNodeHealth::Unhealthy,
        lease_until: Some(lease_until),
        ..Default::default()
    };
    media_port.set_node(tenant, unhealthy_node);

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();
    assert_eq!(report.needs_verification, 1);

    clock.advance(DurationMs::from_millis(15_000));

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();
    assert_eq!(report.migrations_succeeded, 1);
    assert_eq!(report.missing_failed, 0);

    let migrated_session = uow
        .media_session_repository()
        .get(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("session still exists");
    assert_eq!(migrated_session.state(), MediaSessionState::Active);
    let migrated_binding = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("migrated binding exists");
    assert_eq!(migrated_binding.state(), MediaBindingState::Active);
    assert_ne!(migrated_binding.media_binding_id(), binding_id);

    // Clean up temporary database files.
    drop(uow);
    drop(storage);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_media_reconcile_stops_orphan_sessions_on_node_without_local_binding() {
    let file_id = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let base = std::env::temp_dir().join(format!("cheetah_media_reconcile_orphan_stop_{file_id}"));
    let db_path = base.with_extension("db");
    let wal_path = base.with_extension("db-wal");
    let shm_path = base.with_extension("db-shm");

    let storage = Arc::new(SqliteStorage::new(&db_path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<InMemoryClock> = Arc::new(InMemoryClock::new());
    let clock_dyn: Arc<dyn Clock> = clock.clone();
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let owner_resolver: Arc<InMemoryDeviceOwnerResolver> =
        Arc::new(InMemoryDeviceOwnerResolver::new());
    let media_port = Arc::new(InMemoryMediaPort::new(id_gen_dyn.clone()));
    let source_node_id = id_generator.generate_node_id();

    let device_service = DeviceService::new(clock_dyn.clone(), id_gen_dyn.clone());
    let media_service = MediaService::new(
        clock_dyn.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        media_port.clone(),
        source_node_id,
    );

    let tenant = id_generator.generate_tenant_id();
    let channel_id = id_generator.generate_channel_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock_dyn.as_ref());

    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-orphan-stop-1".to_string(),
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
            owner_epoch: OwnerEpoch(1),
            lease_until: None,
        },
    );

    // Start a live session that creates a local binding on node1.
    let session = media_service
        .start_live(
            &ctx,
            &mut *uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel_id.to_string(),
                idempotency_key: "orphan-stop-live-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    uow = storage.begin().await.unwrap();
    let binding = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("binding exists");
    let local_node_id = binding.media_node_id();
    let local_epoch = binding.media_node_instance_epoch();

    // Report the local session on node1 and an orphan session on node2.
    let orphan_session_id = id_generator.generate_media_session_id();
    let orphan_node_id = id_generator.generate_node_id();
    let orphan_epoch = MediaNodeInstanceEpoch(1);

    media_port.set_node_sessions(
        tenant,
        local_node_id,
        vec![MediaNodeSessionRef {
            media_session_id: session.media_session_id,
            device_id: Some(device.device_id),
            channel_id: Some(channel_id),
            media_node_instance_epoch: local_epoch,
        }],
    );
    media_port.set_node_sessions(
        tenant,
        orphan_node_id,
        vec![MediaNodeSessionRef {
            media_session_id: orphan_session_id,
            device_id: Some(device.device_id),
            channel_id: Some(channel_id),
            media_node_instance_epoch: orphan_epoch,
        }],
    );

    let report = media_service.reconcile(&ctx, &mut *uow).await.unwrap();
    uow = storage.begin().await.unwrap();

    assert_eq!(report.orphans_detected, 1);
    assert_eq!(report.orphans_stopped, 1);

    // The local session must still be active and the orphan must be gone from the media port.
    let active_binding = uow
        .media_binding_repository()
        .get_by_media_session(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("binding exists");
    assert_eq!(active_binding.state(), MediaBindingState::Active);

    let orphan_page = media_port
        .list_sessions(
            tenant,
            orphan_node_id,
            PageRequest::new(100).unwrap(),
            clock_dyn.as_ref(),
        )
        .await
        .unwrap();
    assert!(orphan_page.items.is_empty());

    // Clean up temporary database files.
    drop(uow);
    drop(storage);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);
}

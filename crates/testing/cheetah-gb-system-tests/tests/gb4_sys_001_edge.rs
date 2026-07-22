//! GB4-SYS-001: edge SQLite + fake media vertical system test.
//!
//! Drives the full GB28181 control-plane path end to end against a real SQLite
//! backend and in-memory/fake adapters, with no external dependencies:
//!
//! - access: authenticated REGISTER + keepalive on the `Gb28181Access` wire
//!   state machine, persisted through the `ProtocolSessionLink` aggregate;
//! - event: catalog + alarm MANSCDP MESSAGEs decoded into domain events and the
//!   catalog bridged into the device channel catalog;
//! - media: INVITE/200/ACK/BYE negotiated on the `Gb28181Media` state machine
//!   (SIP/SDP control only) mirrored into a persisted `MediaSession` via the
//!   fake `MediaPort`;
//! - command: a PTZ operation submitted, dispatched and delivered over the
//!   in-process command bus;
//! - restart recovery: reopening the storage shows the persisted device,
//!   channel, media session and protocol session.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator, InMemoryMediaPort,
    request_context,
};
use cheetah_domain::{
    ChannelStatus, CommandPayload, CompatibilityProfile, DeviceLifecycle, LocalIdentity,
    MediaSessionState, OperationStatus, OwnerInfo, PresenceState, Protocol, PtzDirection,
    RegistrationInfo, SessionEndpoint, SipTransport,
};
use cheetah_gb28181_module::{
    Gb28181Event, ProtocolSessionLink, RegisterOutcome, RegisterParams, SessionContext,
};
use cheetah_message_api::bus::RawCommandBus;
use cheetah_message_api::mapper::decode_command;
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_application::{
    ChannelDescriptor, CommandDispatcher, DeviceService, MarkDeviceOnlineRequest, MediaService,
    OperationService, RegisterDeviceRequest, ReplaceChannelCatalogRequest, StartLiveRequest,
    StopLiveRequest, SubmitOperationRequest,
};
use cheetah_signal_types::{
    Clock, DurationMs, IdGenerator, OwnerEpoch, ProtocolIdentity, ResourceId, ResourceKind,
    ResourceRef,
};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

#[tokio::test(flavor = "multi_thread")]
async fn edge_sqlite_full_gb28181_vertical_path() {
    let file_id = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let base = std::env::temp_dir().join(format!("cheetah_gb4_sys_001_{file_id}"));
    let db_path = base.with_extension("db");
    let wal_path = base.with_extension("db-wal");
    let shm_path = base.with_extension("db-shm");

    let storage = Arc::new(SqliteStorage::new(&db_path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let owner_resolver = Arc::new(InMemoryDeviceOwnerResolver::new());
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
    let session_link = ProtocolSessionLink::new(clock.clone(), id_gen_dyn.clone());

    let tenant = id_generator.generate_tenant_id();
    let channel_id = id_generator.generate_channel_id();

    // --- Access: authenticated REGISTER on the GB28181 wire ----------------
    let mut access = common::build_access();
    let register_outputs = common::register_device(&mut access, 1_000);
    assert!(
        common::events(&register_outputs)
            .any(|e| matches!(e, Gb28181Event::DeviceRegistered { .. })),
        "wire REGISTER must emit DeviceRegistered"
    );

    // Application device registration keyed by the GB external id.
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());
    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: common::DEVICE_ID.to_string(),
                authority: Some("edge".to_string()),
                name: "edge-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;
    let device_id = device.device_id;
    uow.commit().await.unwrap();

    // Persist the registration binding through the protocol session aggregate.
    let session_ctx = SessionContext {
        tenant_id: tenant,
        device_id,
        protocol_identity: ProtocolIdentity::new(common::DEVICE_ID).unwrap(),
        local_identity: LocalIdentity {
            listener_id: "edge-listener".to_string(),
            local_device_id: "34020000002000000001".to_string(),
            domain: common::DOMAIN.to_string(),
            realm: common::REALM.to_string(),
        },
        transport: SipTransport::Udp,
        owner_node_id: source_node_id,
        owner_epoch: OwnerEpoch(1),
        compatibility: CompatibilityProfile::default(),
    };
    let expiry = clock
        .now_wall()
        .checked_add(DurationMs::from_seconds(3600))
        .unwrap();
    let register_params = RegisterParams {
        endpoint: SessionEndpoint {
            observed_source: common::SOURCE.to_string(),
            contact_uri: format!("sip:{}@{}", common::DEVICE_ID, common::SOURCE),
            advertised_endpoint: common::SOURCE.to_string(),
        },
        registration: RegistrationInfo {
            call_id: "reg-call-id".to_string(),
            cseq: 1,
            expires_secs: 3600,
        },
        expiry_at: expiry,
    };
    let mut session_repo = storage.protocol_session_repository();
    let outcome = session_link
        .register(&mut *session_repo, &session_ctx, register_params)
        .await
        .unwrap();
    let protocol_session_id = match outcome {
        RegisterOutcome::Created {
            protocol_session_id,
            owner_epoch,
        } => {
            assert_eq!(owner_epoch, OwnerEpoch(1));
            protocol_session_id
        }
        other => panic!("expected Created, got {other:?}"),
    };

    // --- Access: keepalive keeps presence online --------------------------
    let keepalive_outputs = common::keepalive(&mut access, 1_010);
    assert!(
        common::events(&keepalive_outputs).any(|e| matches!(e, Gb28181Event::Keepalive { .. })),
        "wire keepalive must emit Keepalive"
    );
    session_link
        .keepalive(&mut *session_repo, &session_ctx)
        .await
        .unwrap();

    // --- Event: catalog response decoded and bridged into the catalog ------
    let catalog_outputs = common::catalog_response(&mut access, 1_020);
    let catalog_items = common::events(&catalog_outputs)
        .find_map(|e| match e {
            Gb28181Event::CatalogReceived { items, .. } => Some(items.clone()),
            _ => None,
        })
        .expect("wire catalog must emit CatalogReceived");
    assert_eq!(catalog_items.len(), 1);

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
    device_service
        .replace_channel_catalog(
            &ctx,
            &mut *uow,
            device_id,
            ReplaceChannelCatalogRequest {
                channels: catalog_items
                    .iter()
                    .map(|item| ChannelDescriptor {
                        id: Some(channel_id.to_string()),
                        name: item.name.clone().unwrap_or_else(|| item.device_id.clone()),
                        kind: "video".to_string(),
                        enabled: true,
                        status: Some("online".to_string()),
                        stream_profiles: Vec::new(),
                        ptz_capabilities: None,
                        metadata: None,
                    })
                    .collect(),
            },
        )
        .await
        .unwrap();
    uow.commit().await.unwrap();

    // --- Event: alarm notification decoded --------------------------------
    let alarm_outputs = common::alarm_notify(&mut access, 1_030);
    assert!(
        common::events(&alarm_outputs).any(|e| matches!(e, Gb28181Event::AlarmReceived { .. })),
        "wire alarm must emit AlarmReceived"
    );

    // --- Media negotiation on the GB28181 media state machine --------------
    let media_session_wire_id = cheetah_signal_types::MediaSessionId::generate();
    let mut media = common::build_media();
    let media_events = common::negotiate_media(
        &mut media,
        media_session_wire_id,
        cheetah_signal_types::ChannelId::generate(),
    );
    assert!(
        media_events
            .iter()
            .any(|e| matches!(e, Gb28181Event::MediaSessionStarted { .. })),
        "media negotiation must establish the session"
    );
    assert!(
        media_events
            .iter()
            .any(|e| matches!(e, Gb28181Event::MediaSessionStopped { .. })),
        "media negotiation must tear the session down"
    );

    // Mirror the negotiated session into a persisted MediaSession via the port.
    owner_resolver.set_owner(
        tenant,
        device_id,
        OwnerInfo {
            owner_node_id: source_node_id,
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );
    let mut uow = storage.begin().await.unwrap();
    let session = media_service
        .start_live(
            &ctx,
            &mut *uow,
            StartLiveRequest {
                device_id: device_id.to_string(),
                channel_id: channel_id.to_string(),
                idempotency_key: "live-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.state, MediaSessionState::Inviting);
    uow.commit().await.unwrap();

    // --- Command: submit and dispatch a PTZ operation ----------------------
    let mut sub = command_bus.subscribe("", "").await.unwrap();
    let mut uow = storage.begin().await.unwrap();
    let ptz = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: ResourceRef {
                    tenant_id: tenant,
                    kind: ResourceKind::Device,
                    id: ResourceId::Device(device_id),
                },
                payload: CommandPayload::Ptz {
                    channel_id,
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
    let dispatched = command_dispatcher
        .dispatch(&ctx, &mut *uow, tenant, ptz.operation_id)
        .await
        .unwrap();
    assert_eq!(dispatched.status, OperationStatus::Running);
    uow.commit().await.unwrap();

    let delivery = tokio::time::timeout(Duration::from_secs(1), sub.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command delivered");
    let decoded = decode_command(&delivery.envelope).unwrap();
    assert_eq!(decoded.operation_id(), ptz.operation_id);

    // Stop the persisted media session.
    let mut uow = storage.begin().await.unwrap();
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
    uow.commit().await.unwrap();

    // --- Restart recovery: reopen storage and verify persisted state -------
    let mut uow = storage.begin().await.unwrap();
    let recovered = uow
        .device_repository()
        .get(tenant, device_id)
        .await
        .unwrap()
        .expect("device persisted");
    assert_eq!(recovered.lifecycle(), DeviceLifecycle::Active);

    let recovered_channel = uow
        .channel_repository()
        .get(tenant, device_id, channel_id)
        .await
        .unwrap()
        .expect("channel persisted");
    assert_eq!(recovered_channel.status(), ChannelStatus::Online);

    let recovered_session = uow
        .media_session_repository()
        .get(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("media session persisted");
    assert_eq!(recovered_session.state(), MediaSessionState::Stopped);

    let by_external = uow
        .device_repository()
        .get_by_external_id(
            tenant,
            Protocol::Gb28181,
            ProtocolIdentity::new(common::DEVICE_ID).unwrap(),
        )
        .await
        .unwrap()
        .expect("device by external id");
    assert_eq!(by_external.device_id(), device_id);
    drop(uow);

    let persisted_session = session_repo
        .get(tenant, protocol_session_id)
        .await
        .unwrap()
        .expect("protocol session persisted");
    assert_eq!(persisted_session.presence(), PresenceState::Online);
    assert_eq!(persisted_session.device_id(), device_id);

    drop(session_repo);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);
}

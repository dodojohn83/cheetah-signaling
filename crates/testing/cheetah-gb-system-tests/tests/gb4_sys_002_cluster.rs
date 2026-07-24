//! GB4-SYS-002: cluster PostgreSQL + NATS + fake media vertical system test.
//!
//! Boots disposable PostgreSQL and NATS containers (dynamic host ports) and
//! drives a multi-node GB28181 control-plane vertical:
//!
//! - access: authenticated REGISTER + keepalive on the `Gb28181Access` wire,
//!   persisted through the `ProtocolSessionLink` aggregate on PostgreSQL;
//! - media: INVITE/200/ACK/BYE negotiated on `Gb28181Media` (SIP/SDP control
//!   only), mirrored into a persisted `MediaSession` via the fake `MediaPort`;
//! - command routing: a command dispatched from node B is delivered over NATS
//!   to the current owner (node A);
//! - takeover + fencing: after node A's lease expires, node B acquires a higher
//!   owner epoch; a stale-epoch keepalive is rejected and post-takeover commands
//!   route only to node B.
//!
//! No RTP/RTCP/PS/TS/ES media payload is produced. Containers are torn down at
//! the end and no developer database or public infrastructure is used.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use cheetah_cluster_ownership::{CachingDeviceOwnerResolver, OwnerLeaseService};
use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryIdGenerator, InMemoryMediaPort, request_context,
};
use cheetah_domain::{
    CommandPayload, CompatibilityProfile, DeviceOwnerResolver, IdGenerator, LocalIdentity,
    MediaSessionState, PresenceState, PtzDirection, RegistrationInfo, SessionEndpoint,
    SipTransport,
};
use cheetah_gb28181_module::{
    Gb28181Event, ProtocolSessionLink, RegisterOutcome, RegisterParams, SessionContext,
    SessionLinkError,
};
use cheetah_message_api::{RawCommandBus, command_subject, decode_command};
use cheetah_message_nats::NatsBus;
use cheetah_signal_application::{
    ChannelDescriptor, CommandDispatcher, DeviceService, MarkDeviceOnlineRequest, MediaService,
    OperationService, RegisterDeviceRequest, ReplaceChannelCatalogRequest, StartLiveRequest,
    SubmitOperationRequest,
};
use cheetah_signal_types::{
    Clock, DurationMs, OwnerEpoch, ProtocolIdentity, ResourceId, ResourceKind, ResourceRef,
};
use cheetah_storage_api::{OwnerRepository, Storage};
use cheetah_storage_postgres::{PostgresOwnerRepository, PostgresStorage};
use testcontainers_modules::nats::{Nats, NatsServerCmd};
use testcontainers_modules::postgres;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

#[tokio::test(flavor = "multi_thread")]
async fn cluster_postgres_nats_full_gb28181_vertical_and_takeover() {
    let pg_container = postgres::Postgres::default().start().await.unwrap();
    let nats_cmd = NatsServerCmd::default().with_jetstream();
    let nats_container = Nats::default().with_cmd(&nats_cmd).start().await.unwrap();

    let pg_host = pg_container.get_host().await.unwrap();
    let pg_port = pg_container.get_host_port_ipv4(5432).await.unwrap();
    let nats_host = nats_container.get_host().await.unwrap();
    let nats_port = nats_container.get_host_port_ipv4(4222).await.unwrap();

    let pg_url =
        format!("postgres://postgres:postgres@{pg_host}:{pg_port}/postgres?sslmode=disable");
    let nats_url = format!("nats://{nats_host}:{nats_port}");

    let storage = Arc::new(PostgresStorage::new(&pg_url).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock_impl = Arc::new(InMemoryClock::new());
    let clock: Arc<dyn Clock> = clock_impl.clone();
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let tenant = id_generator.generate_tenant_id();
    let channel_id = id_generator.generate_channel_id();
    let node_a = id_generator.generate_node_id();
    let node_b = id_generator.generate_node_id();

    let lease_repo_a: Arc<dyn OwnerRepository> = Arc::new(PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
    ));
    let lease_repo_b: Arc<dyn OwnerRepository> = Arc::new(PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
    ));

    let resolver_repo: Arc<dyn OwnerRepository> = Arc::new(PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
    ));
    let owner_resolver: Arc<dyn DeviceOwnerResolver> = Arc::new(CachingDeviceOwnerResolver::new(
        resolver_repo,
        clock.clone(),
        DurationMs::from_millis(1),
        128,
    ));

    let lease_a = OwnerLeaseService::new(
        lease_repo_a,
        clock.clone(),
        node_a,
        DurationMs::from_millis(100),
    );
    let lease_b = OwnerLeaseService::new(
        lease_repo_b,
        clock.clone(),
        node_b,
        DurationMs::from_millis(100),
    );

    let nats_a = Arc::new(
        NatsBus::connect_insecure(
            &nats_url,
            node_a,
            owner_resolver.clone(),
            Duration::from_secs(5),
            Duration::from_secs(2),
        )
        .await
        .unwrap(),
    );
    let nats_b = Arc::new(
        NatsBus::connect_insecure(
            &nats_url,
            node_b,
            owner_resolver.clone(),
            Duration::from_secs(5),
            Duration::from_secs(2),
        )
        .await
        .unwrap(),
    );

    let dispatcher_b = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        nats_b.clone(),
    );

    let device_service = DeviceService::new(clock.clone(), id_gen_dyn.clone());
    let operation_service = OperationService::new(clock.clone(), id_gen_dyn.clone());
    let media_port: Arc<dyn cheetah_domain::MediaPort> =
        Arc::new(InMemoryMediaPort::new(id_gen_dyn.clone()));
    let media_service = MediaService::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        media_port,
        node_a,
    );
    let session_link = ProtocolSessionLink::new(clock.clone(), id_gen_dyn.clone());

    // --- Access: authenticated REGISTER + keepalive on the GB28181 wire -----
    let mut access = common::build_access();
    let register_outputs = common::register_device(&mut access, 1_000);
    assert!(
        common::events(&register_outputs)
            .any(|e| matches!(e, Gb28181Event::DeviceRegistered { .. })),
        "wire REGISTER must emit DeviceRegistered"
    );
    let keepalive_outputs = common::keepalive(&mut access, 1_010);
    assert!(
        common::events(&keepalive_outputs).any(|e| matches!(e, Gb28181Event::Keepalive { .. })),
        "wire keepalive must emit Keepalive"
    );

    // --- Application registration + channel catalog -------------------------
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());
    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: common::DEVICE_ID.to_string(),
                authority: Some("cluster".to_string()),
                name: "cluster-camera".to_string(),
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
    uow.commit().await.unwrap();

    // --- Node A acquires ownership (epoch 1) --------------------------------
    let owner_a = lease_a.acquire(tenant, device_id).await.unwrap();
    assert_eq!(owner_a.owner_node_id, node_a);
    assert_eq!(owner_a.owner_epoch, OwnerEpoch(1));

    // Persist the registration binding fenced by the current owner epoch.
    let ctx_a = session_ctx(tenant, device_id, node_a, OwnerEpoch(1));
    let expiry = clock
        .now_wall()
        .checked_add(DurationMs::from_seconds(3600))
        .unwrap();
    let mut session_repo = storage.protocol_session_repository();
    let outcome = session_link
        .register(&mut *session_repo, &ctx_a, register_params(expiry))
        .await
        .unwrap();
    let protocol_session_id = match outcome {
        RegisterOutcome::Created {
            protocol_session_id,
            ..
        } => protocol_session_id,
        other => panic!("expected Created, got {other:?}"),
    };
    session_link
        .keepalive(&mut *session_repo, &ctx_a)
        .await
        .unwrap();

    // --- Media negotiation (wire) + persisted MediaSession via fake port ----
    let mut media = common::build_media();
    let media_events = common::negotiate_media(
        &mut media,
        cheetah_signal_types::MediaSessionId::generate(),
        cheetah_signal_types::ChannelId::generate(),
    );
    assert!(
        media_events
            .iter()
            .any(|e| matches!(e, Gb28181Event::MediaSessionStarted { .. })),
        "media negotiation must establish the session"
    );

    let mut uow = storage.begin().await.unwrap();
    let session = media_service
        .start_live(
            &ctx,
            &mut *uow,
            StartLiveRequest {
                device_id: device_id.to_string(),
                channel_id: channel_id.to_string(),
                idempotency_key: "live-cluster-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.state, MediaSessionState::Inviting);
    uow.commit().await.unwrap();

    // --- Command routing: node B dispatches, node A (owner) receives --------
    let subject_a = command_subject(tenant, node_a);
    let subject_b = command_subject(tenant, node_b);
    let mut sub_a = nats_a.subscribe(&subject_a, "consumer-a").await.unwrap();
    let mut sub_b = nats_b.subscribe(&subject_b, "consumer-b").await.unwrap();

    let ptz = submit_ptz(
        &operation_service,
        &ctx,
        &storage,
        tenant,
        device_id,
        channel_id,
        "ptz-node-a",
        owner_a.owner_epoch,
        &dispatcher_b,
    )
    .await;
    let delivery = tokio::time::timeout(Duration::from_secs(2), sub_a.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command delivered to node A");
    assert_eq!(
        decode_command(&delivery.envelope).unwrap().operation_id(),
        ptz
    );
    delivery.ack.ack().await.unwrap();

    // --- Takeover: node A lease expires, node B acquires epoch 2 ------------
    clock_impl.advance(DurationMs::from_millis(101));
    let owner_b = lease_b.acquire(tenant, device_id).await.unwrap();
    assert_eq!(owner_b.owner_node_id, node_b);
    assert_eq!(owner_b.owner_epoch, OwnerEpoch(2));

    // Fencing: a stale-epoch keepalive is rejected before the session is
    // re-owned, then node B takes the session over at the higher epoch.
    let ctx_b = session_ctx(tenant, device_id, node_b, OwnerEpoch(2));
    // Before takeover the epoch-1 keepalive is still valid.
    session_link
        .keepalive(&mut *session_repo, &ctx_a)
        .await
        .unwrap();
    session_link
        .acquire_owner(&mut *session_repo, tenant, device_id, node_b, OwnerEpoch(2))
        .await
        .unwrap()
        .expect("session taken over");
    let fenced = session_link
        .keepalive(&mut *session_repo, &ctx_a)
        .await
        .unwrap_err();
    assert!(
        matches!(fenced, SessionLinkError::StaleOwner { .. }),
        "stale owner epoch must be fenced, got {fenced:?}"
    );
    session_link
        .keepalive(&mut *session_repo, &ctx_b)
        .await
        .unwrap();

    // --- Post-takeover command routes only to node B -----------------------
    let ptz2 = submit_ptz(
        &operation_service,
        &ctx,
        &storage,
        tenant,
        device_id,
        channel_id,
        "ptz-node-b",
        owner_b.owner_epoch,
        &dispatcher_b,
    )
    .await;
    let delivery_b = tokio::time::timeout(Duration::from_secs(2), sub_b.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command delivered to node B after takeover");
    assert_eq!(
        decode_command(&delivery_b.envelope).unwrap().operation_id(),
        ptz2
    );
    delivery_b.ack.ack().await.unwrap();

    let no_delivery = tokio::time::timeout(Duration::from_millis(100), sub_a.next()).await;
    assert!(
        no_delivery.is_err(),
        "node A must not receive the post-takeover command"
    );

    // --- Recovery: persisted state reflects the new owner -------------------
    let persisted = session_repo
        .get(tenant, protocol_session_id)
        .await
        .unwrap()
        .expect("protocol session persisted");
    assert_eq!(persisted.owner_epoch(), OwnerEpoch(2));
    assert_eq!(persisted.presence(), PresenceState::Online);

    let mut uow = storage.begin().await.unwrap();
    let recovered_session = uow
        .media_session_repository()
        .get(tenant, session.media_session_id)
        .await
        .unwrap()
        .expect("media session persisted");
    assert_eq!(recovered_session.device_id(), device_id);
    drop(uow);

    drop(session_repo);
    Arc::try_unwrap(storage)
        .expect("no outstanding storage references")
        .close()
        .await
        .unwrap();
}

fn session_ctx(
    tenant: cheetah_signal_types::TenantId,
    device_id: cheetah_signal_types::DeviceId,
    owner_node_id: cheetah_signal_types::NodeId,
    owner_epoch: OwnerEpoch,
) -> SessionContext {
    SessionContext {
        tenant_id: tenant,
        device_id,
        protocol_identity: ProtocolIdentity::new(common::DEVICE_ID).unwrap(),
        local_identity: LocalIdentity {
            listener_id: "cluster-listener".to_string(),
            local_device_id: "34020000002000000001".to_string(),
            domain: common::DOMAIN.to_string(),
            realm: common::REALM.to_string(),
        },
        transport: SipTransport::Udp,
        owner_node_id,
        owner_epoch,
        compatibility: CompatibilityProfile::default(),
    }
}

fn register_params(expiry: cheetah_signal_types::UtcTimestamp) -> RegisterParams {
    RegisterParams {
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
    }
}

#[allow(clippy::too_many_arguments)]
async fn submit_ptz(
    operation_service: &OperationService,
    ctx: &cheetah_signal_types::RequestContext,
    storage: &Arc<PostgresStorage>,
    tenant: cheetah_signal_types::TenantId,
    device_id: cheetah_signal_types::DeviceId,
    channel_id: cheetah_signal_types::ChannelId,
    idempotency_key: &str,
    expected_owner_epoch: OwnerEpoch,
    dispatcher: &CommandDispatcher,
) -> cheetah_signal_types::OperationId {
    let mut uow = storage.begin().await.unwrap();
    let op = operation_service
        .submit_operation(
            ctx,
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
                idempotency_key: idempotency_key.to_string(),
                deadline: None,
                expected_owner_epoch,
            },
        )
        .await
        .unwrap();
    dispatcher
        .dispatch(ctx, &mut *uow, tenant, op.operation_id)
        .await
        .unwrap();
    uow.commit().await.unwrap();
    op.operation_id
}

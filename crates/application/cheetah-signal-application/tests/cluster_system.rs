//! Multi-node PostgreSQL+NATS system scenario.
//!
//! Covers device distribution across nodes, command routing to the current
//! owner, and owner takeover after a lease expires.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use cheetah_cluster_ownership::{CachingDeviceOwnerResolver, OwnerLeaseService};
use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, request_context};
use cheetah_domain::{CommandPayload, DeviceOwnerResolver, IdGenerator, PtzDirection};
use cheetah_message_api::{RawCommandBus, command_subject, decode_command};
use cheetah_message_nats::NatsBus;
use cheetah_signal_application::{
    ChannelDescriptor, CommandDispatcher, DeviceService, OperationService, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest, SubmitOperationRequest,
};
use cheetah_signal_types::{Clock, DurationMs, OwnerEpoch, ResourceId, ResourceKind, ResourceRef};
use cheetah_storage_api::{OwnerRepository, Storage};
use cheetah_storage_postgres::{PostgresOwnerRepository, PostgresStorage};
use testcontainers_modules::nats::{Nats, NatsServerCmd};
use testcontainers_modules::postgres;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

#[tokio::test(flavor = "multi_thread")]
async fn multi_node_postgres_nats_command_routing_and_takeover() {
    // Start PostgreSQL and NATS containers.
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

    // Build an owner repository and resolver shared by both nodes.
    let pg_repo_a = PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
        clock.clone(),
    );
    let pg_repo_b = pg_repo_a.clone();
    let owner_repo_a: Box<dyn OwnerRepository> = Box::new(pg_repo_a);
    let owner_repo_b: Box<dyn OwnerRepository> = Box::new(pg_repo_b);
    let lease_repo_a = Arc::new(tokio::sync::Mutex::new(owner_repo_a));
    let lease_repo_b = Arc::new(tokio::sync::Mutex::new(owner_repo_b));

    let resolver_repo: Arc<dyn OwnerRepository> = Arc::new(PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
        clock.clone(),
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

    // Create a NATS bus for each node.
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

    let _dispatcher_a = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        nats_a.clone(),
    );
    let dispatcher_b = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        nats_b.clone(),
    );

    let device_service = DeviceService::new(clock.clone(), id_gen_dyn.clone());
    let operation_service = OperationService::new(clock.clone(), id_gen_dyn.clone());

    // Register a device with a channel.
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());
    let mut uow = storage.begin().await.unwrap();
    let device = device_service
        .register_or_update_device(
            &ctx,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-cluster".to_string(),
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
    let device_id = device.device_id;
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

    // Node A acquires the device.
    let owner_a = lease_a.acquire(tenant, device_id).await.unwrap();
    assert_eq!(owner_a.owner_node_id, node_a);
    assert_eq!(owner_a.owner_epoch, OwnerEpoch(1));

    // Subscribe to command subjects for both nodes.
    let subject_a = command_subject(tenant, node_a);
    let subject_b = command_subject(tenant, node_b);
    let mut sub_a = nats_a.subscribe(&subject_a, "consumer-a").await.unwrap();
    let mut sub_b = nats_b.subscribe(&subject_b, "consumer-b").await.unwrap();

    // Submit and dispatch a PTZ command from node B while node A owns the device.
    let ptz_target = ResourceRef {
        tenant_id: tenant,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    };
    let mut uow = storage.begin().await.unwrap();
    let ptz = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: ptz_target,
                payload: CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-node-a".to_string(),
                deadline: None,
                expected_owner_epoch: owner_a.owner_epoch,
            },
        )
        .await
        .unwrap();

    dispatcher_b
        .dispatch(&ctx, &mut *uow, tenant, ptz.operation_id)
        .await
        .unwrap();

    // Node A should receive the command.
    let delivery = tokio::time::timeout(Duration::from_secs(2), sub_a.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command delivered to node A");
    let decoded = decode_command(&delivery.envelope).unwrap();
    assert_eq!(decoded.operation_id(), ptz.operation_id);
    delivery.ack.ack().await.unwrap();

    // Let node A's lease expire and let node B take over.
    clock_impl.advance(DurationMs::from_millis(101));
    let owner_b = lease_b.acquire(tenant, device_id).await.unwrap();
    assert_eq!(owner_b.owner_node_id, node_b);
    assert_eq!(owner_b.owner_epoch, OwnerEpoch(2));

    // Submit another command and route it to the new owner.
    let ptz_target2 = ResourceRef {
        tenant_id: tenant,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    };
    let mut uow = storage.begin().await.unwrap();
    let ptz2 = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: ptz_target2,
                payload: CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-node-b".to_string(),
                deadline: None,
                expected_owner_epoch: owner_b.owner_epoch,
            },
        )
        .await
        .unwrap();

    dispatcher_b
        .dispatch(&ctx, &mut *uow, tenant, ptz2.operation_id)
        .await
        .unwrap();

    let delivery_b = tokio::time::timeout(Duration::from_secs(2), sub_b.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command delivered to node B after takeover");
    let decoded_b = decode_command(&delivery_b.envelope).unwrap();
    assert_eq!(decoded_b.operation_id(), ptz2.operation_id);
    delivery_b.ack.ack().await.unwrap();

    // Node A should not receive anything on its second next call.
    let no_delivery = tokio::time::timeout(Duration::from_millis(100), sub_a.next()).await;
    assert!(
        no_delivery.is_err(),
        "node A must not receive the post-takeover command"
    );

    Arc::try_unwrap(storage)
        .expect("no outstanding storage references")
        .close()
        .await
        .unwrap();
}

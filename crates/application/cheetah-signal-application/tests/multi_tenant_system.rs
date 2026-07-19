//! Multi-tenant PostgreSQL+NATS system scenario.
//!
//! Verifies that tenants with the same external device ID are strictly isolated:
//! different devices, independent ownership, separate command subjects and no
//! cross-tenant command leakage.

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
async fn multi_tenant_postgres_nats_isolation() {
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

    let tenant_a = id_generator.generate_tenant_id();
    let tenant_b = id_generator.generate_tenant_id();
    let channel_id = id_generator.generate_channel_id();
    let node_a = id_generator.generate_node_id();
    let node_b = id_generator.generate_node_id();

    let owner_repo_a = Arc::new(tokio::sync::Mutex::new(PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
    ))) as Arc<tokio::sync::Mutex<dyn OwnerRepository>>;
    let owner_repo_b = Arc::new(tokio::sync::Mutex::new(PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
    ))) as Arc<tokio::sync::Mutex<dyn OwnerRepository>>;
    let lease_repo_a = owner_repo_a.clone();
    let lease_repo_b = owner_repo_b.clone();

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

    let nats = Arc::new(
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

    let dispatcher = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        nats.clone(),
    );

    let device_service = DeviceService::new(clock.clone(), id_gen_dyn.clone());
    let operation_service = OperationService::new(clock.clone(), id_gen_dyn.clone());

    let ctx_a = request_context(tenant_a, id_generator.as_ref(), clock.as_ref());
    let ctx_b = request_context(tenant_b, id_generator.as_ref(), clock.as_ref());

    // Both tenants register a device with the same external ID.
    let mut uow = storage.begin().await.unwrap();
    let device_a = device_service
        .register_or_update_device(
            &ctx_a,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-shared".to_string(),
                authority: Some("auth".to_string()),
                name: "shared-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;
    let device_b = device_service
        .register_or_update_device(
            &ctx_b,
            &mut *uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-shared".to_string(),
                authority: Some("auth".to_string()),
                name: "shared-camera".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;

    assert_ne!(
        device_a.device_id, device_b.device_id,
        "same external id must produce distinct devices per tenant"
    );

    // Add the same channel id to each device.
    let mut uow = storage.begin().await.unwrap();
    device_service
        .replace_channel_catalog(
            &ctx_a,
            &mut *uow,
            device_a.device_id,
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
        .replace_channel_catalog(
            &ctx_b,
            &mut *uow,
            device_b.device_id,
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

    // Different nodes acquire ownership per tenant.
    let owner_a = lease_a.acquire(tenant_a, device_a.device_id).await.unwrap();
    let owner_b = lease_b.acquire(tenant_b, device_b.device_id).await.unwrap();
    assert_eq!(owner_a.owner_node_id, node_a);
    assert_eq!(owner_b.owner_node_id, node_b);
    assert_eq!(owner_a.owner_epoch, OwnerEpoch(1));
    assert_eq!(owner_b.owner_epoch, OwnerEpoch(1));

    // Subscribe to the per-node command subjects.
    let subject_a = command_subject(tenant_a, node_a);
    let subject_b = command_subject(tenant_b, node_b);
    let mut sub_a = nats.subscribe(&subject_a, "consumer-a").await.unwrap();
    let mut sub_b = nats.subscribe(&subject_b, "consumer-b").await.unwrap();

    // Dispatch a PTZ command for tenant A.
    let mut uow = storage.begin().await.unwrap();
    let ptz_a = operation_service
        .submit_operation(
            &ctx_a,
            &mut *uow,
            SubmitOperationRequest {
                device_id: device_a.device_id,
                target: ResourceRef {
                    tenant_id: tenant_a,
                    kind: ResourceKind::Device,
                    id: ResourceId::Device(device_a.device_id),
                },
                payload: CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-tenant-a".to_string(),
                deadline: None,
                expected_owner_epoch: owner_a.owner_epoch,
            },
        )
        .await
        .unwrap();
    dispatcher
        .dispatch(&ctx_a, &mut *uow, tenant_a, ptz_a.operation_id)
        .await
        .unwrap();

    // Dispatch a PTZ command for tenant B.
    let ptz_b = operation_service
        .submit_operation(
            &ctx_b,
            &mut *uow,
            SubmitOperationRequest {
                device_id: device_b.device_id,
                target: ResourceRef {
                    tenant_id: tenant_b,
                    kind: ResourceKind::Device,
                    id: ResourceId::Device(device_b.device_id),
                },
                payload: CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-tenant-b".to_string(),
                deadline: None,
                expected_owner_epoch: owner_b.owner_epoch,
            },
        )
        .await
        .unwrap();
    dispatcher
        .dispatch(&ctx_b, &mut *uow, tenant_b, ptz_b.operation_id)
        .await
        .unwrap();

    // Each tenant receives only its own command on the matching subject.
    let delivery_a = tokio::time::timeout(Duration::from_secs(2), sub_a.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command delivered to tenant A");
    let decoded_a = decode_command(&delivery_a.envelope).unwrap();
    assert_eq!(decoded_a.operation_id(), ptz_a.operation_id);
    assert_eq!(decoded_a.tenant_id(), tenant_a);
    delivery_a.ack.ack().await.unwrap();

    let delivery_b = tokio::time::timeout(Duration::from_secs(2), sub_b.next())
        .await
        .unwrap()
        .unwrap()
        .expect("command delivered to tenant B");
    let decoded_b = decode_command(&delivery_b.envelope).unwrap();
    assert_eq!(decoded_b.operation_id(), ptz_b.operation_id);
    assert_eq!(decoded_b.tenant_id(), tenant_b);
    delivery_b.ack.ack().await.unwrap();

    // A tenant A subscriber must not see a tenant B command.
    let no_leak = tokio::time::timeout(Duration::from_millis(100), sub_a.next()).await;
    assert!(
        no_leak.is_err(),
        "tenant A must not receive tenant B command"
    );

    Arc::try_unwrap(storage)
        .expect("no outstanding storage references")
        .close()
        .await
        .unwrap();
}

//! PostgreSQL/NATS brief interruption and recovery system scenario.
//!
//! Verifies that the storage pool and NATS command bus can recover after a
//! transient container outage: operations fail or time out while the backend
//! is down, and succeed again once it is restarted.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use cheetah_cluster_ownership::{CachingDeviceOwnerResolver, OwnerLeaseService};
use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, request_context};
use cheetah_domain::{DeviceOwnerResolver, IdGenerator, PtzDirection};
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
async fn postgres_nats_brief_interruption_and_recovery() {
    // Start PostgreSQL and NATS containers.
    let pg_container = postgres::Postgres::default().start().await.unwrap();
    let nats_cmd = NatsServerCmd::default().with_jetstream();
    let nats_container = Nats::default().with_cmd(&nats_cmd).start().await.unwrap();

    let pg_host = pg_container.get_host().await.unwrap().to_string();
    let mut pg_port = pg_container.get_host_port_ipv4(5432).await.unwrap();
    let nats_host = nats_container.get_host().await.unwrap().to_string();
    let mut nats_port = nats_container.get_host_port_ipv4(4222).await.unwrap();

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

    let owner_repo = PostgresOwnerRepository::new(
        storage.read_pool().clone(),
        storage.write_pool().clone(),
        clock.clone(),
    );
    let owner_repo_for_resolver: Arc<dyn OwnerRepository> = Arc::new(owner_repo.clone());
    let resolver: Arc<dyn DeviceOwnerResolver> = Arc::new(CachingDeviceOwnerResolver::new(
        owner_repo_for_resolver.clone(),
        clock.clone(),
        DurationMs::from_millis(1),
        128,
    ));

    let lease_service = OwnerLeaseService::new(
        Arc::new(tokio::sync::Mutex::new(
            Box::new(owner_repo.clone()) as Box<dyn OwnerRepository>
        )),
        clock.clone(),
        node_a,
        DurationMs::from_millis(100),
    );

    let nats = Arc::new(
        NatsBus::connect_insecure(
            &nats_url,
            node_b,
            resolver.clone(),
            Duration::from_secs(5),
            Duration::from_secs(2),
        )
        .await
        .unwrap(),
    );

    let dispatcher = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        resolver.clone(),
        nats.clone(),
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
                external_id: "ext-interrupt".to_string(),
                authority: Some("auth".to_string()),
                name: "interrupt-camera".to_string(),
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

    // Acquire the device and submit a baseline command.
    let owner = lease_service.acquire(tenant, device_id).await.unwrap();
    assert_eq!(owner.owner_node_id, node_a);

    let subject_a = command_subject(tenant, node_a);
    let mut subscriber = nats
        .subscribe(&subject_a, "consumer-interrupt")
        .await
        .unwrap();

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
                target: ptz_target.clone(),
                payload: cheetah_domain::CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-baseline".to_string(),
                deadline: None,
                expected_owner_epoch: owner.owner_epoch,
            },
        )
        .await
        .unwrap();
    dispatcher
        .dispatch(&ctx, &mut *uow, tenant, ptz.operation_id)
        .await
        .unwrap();

    let delivery = tokio::time::timeout(Duration::from_secs(2), subscriber.next())
        .await
        .unwrap()
        .unwrap()
        .expect("baseline command delivered");
    let decoded = decode_command(&delivery.envelope).unwrap();
    assert_eq!(decoded.operation_id(), ptz.operation_id);
    delivery.ack.ack().await.unwrap();

    // Interrupt NATS: wait until the port is actually closed, then dispatch.
    nats_container.stop().await.unwrap();
    wait_until_unreachable(&nats_host, nats_port, Duration::from_secs(5)).await;

    let mut uow = storage.begin().await.unwrap();
    let ptz_interrupted = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: ptz_target.clone(),
                payload: cheetah_domain::CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-interrupted".to_string(),
                deadline: None,
                expected_owner_epoch: owner.owner_epoch,
            },
        )
        .await
        .unwrap();
    let interrupted_dto = dispatcher
        .dispatch(&ctx, &mut *uow, tenant, ptz_interrupted.operation_id)
        .await
        .unwrap();
    assert_eq!(
        interrupted_dto.status,
        cheetah_domain::OperationStatus::Failed
    );
    let result = interrupted_dto
        .result
        .expect("operation result present after outage dispatch");
    assert_eq!(result.error_code(), Some("COMMAND_BUS"));

    // Restart NATS and create a fresh bus/subscription to recreate JetStream streams.
    nats_container.start().await.unwrap();
    nats_port = nats_container.get_host_port_ipv4(4222).await.unwrap();
    let nats_url = format!("nats://{nats_host}:{nats_port}");
    wait_until_reachable(&nats_host, nats_port, Duration::from_secs(5)).await;

    let nats_recovered = Arc::new(
        wait_for_nats_ready(&nats_url, node_b, resolver.clone(), Duration::from_secs(10))
            .await
            .unwrap(),
    );
    let dispatcher_recovered = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        resolver.clone(),
        nats_recovered.clone(),
    );
    let mut subscriber_recovered = nats_recovered
        .subscribe(&subject_a, "consumer-interrupt-recovered")
        .await
        .unwrap();

    let mut uow = storage.begin().await.unwrap();
    let ptz_recovered = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: ptz_target.clone(),
                payload: cheetah_domain::CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-recovered".to_string(),
                deadline: None,
                expected_owner_epoch: owner.owner_epoch,
            },
        )
        .await
        .unwrap();
    dispatcher_recovered
        .dispatch(&ctx, &mut *uow, tenant, ptz_recovered.operation_id)
        .await
        .unwrap();

    let delivery_recovered = wait_for_command(
        &mut subscriber_recovered,
        ptz_recovered.operation_id,
        Duration::from_secs(5),
    )
    .await
    .expect("command delivered after NATS recovery");
    let decoded_recovered = decode_command(&delivery_recovered.envelope).unwrap();
    assert_eq!(decoded_recovered.operation_id(), ptz_recovered.operation_id);
    delivery_recovered.ack.ack().await.unwrap();

    // Interrupt PostgreSQL: storage queries should fail while down, then recover.
    pg_container.stop().await.unwrap();
    wait_until_unreachable(&pg_host, pg_port, Duration::from_secs(5)).await;

    let pg_during_outage = PostgresStorage::new(&pg_url).await;
    assert!(
        pg_during_outage.is_err(),
        "new storage connection must fail while PostgreSQL is stopped"
    );

    pg_container.start().await.unwrap();
    pg_port = pg_container.get_host_port_ipv4(5432).await.unwrap();
    let pg_url =
        format!("postgres://postgres:postgres@{pg_host}:{pg_port}/postgres?sslmode=disable");
    wait_until_reachable(&pg_host, pg_port, Duration::from_secs(5)).await;

    // Reconnect storage and rebuild the resolver/dispatcher on the recovered ports.
    let storage_recovered = Arc::new(
        wait_for_postgres_ready(&pg_url, Duration::from_secs(10))
            .await
            .unwrap(),
    );
    let owner_repo_recovered = PostgresOwnerRepository::new(
        storage_recovered.read_pool().clone(),
        storage_recovered.write_pool().clone(),
        clock.clone(),
    );
    let resolver_recovered: Arc<dyn DeviceOwnerResolver> =
        Arc::new(CachingDeviceOwnerResolver::new(
            Arc::new(owner_repo_recovered.clone()) as Arc<dyn OwnerRepository>,
            clock.clone(),
            DurationMs::from_millis(1),
            128,
        ));

    let recovered_owner = owner_repo_recovered
        .get(tenant, device_id)
        .await
        .unwrap()
        .expect("owner row recovered from PostgreSQL");
    assert_eq!(recovered_owner.owner_node_id, node_a);
    assert_eq!(recovered_owner.owner_epoch, OwnerEpoch(1));

    // A final operation after both backends are up should dispatch end-to-end.
    let dispatcher_final = CommandDispatcher::new(
        clock.clone(),
        id_gen_dyn.clone(),
        resolver_recovered,
        nats_recovered.clone(),
    );
    let mut uow = storage_recovered.begin().await.unwrap();
    let ptz_final = operation_service
        .submit_operation(
            &ctx,
            &mut *uow,
            SubmitOperationRequest {
                device_id,
                target: ptz_target,
                payload: cheetah_domain::CommandPayload::Ptz {
                    channel_id,
                    direction: PtzDirection::Stop,
                    speed: 0.0,
                },
                idempotency_key: "ptz-final".to_string(),
                deadline: None,
                expected_owner_epoch: owner.owner_epoch,
            },
        )
        .await
        .unwrap();
    dispatcher_final
        .dispatch(&ctx, &mut *uow, tenant, ptz_final.operation_id)
        .await
        .unwrap();

    let delivery_final = wait_for_command(
        &mut subscriber_recovered,
        ptz_final.operation_id,
        Duration::from_secs(5),
    )
    .await
    .expect("final command delivered after full recovery");
    let decoded_final = decode_command(&delivery_final.envelope).unwrap();
    assert_eq!(decoded_final.operation_id(), ptz_final.operation_id);
    delivery_final.ack.ack().await.unwrap();

    Arc::try_unwrap(storage)
        .expect("no outstanding storage references")
        .close()
        .await
        .unwrap();
    Arc::try_unwrap(storage_recovered)
        .expect("no outstanding storage references")
        .close()
        .await
        .unwrap();
}

async fn wait_until_unreachable(host: &str, port: u16, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() >= timeout {
            panic!("port {host}:{port} remained reachable within {timeout:?}");
        }
        let attempt = tokio::time::timeout(
            Duration::from_millis(200),
            tokio::net::TcpStream::connect((host, port)),
        );
        match attempt.await {
            Ok(Ok(_)) => tokio::time::sleep(Duration::from_millis(50)).await,
            Ok(Err(_)) | Err(_) => break,
        }
    }
}

async fn wait_until_reachable(host: &str, port: u16, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() >= timeout {
            panic!("port {host}:{port} remained unreachable within {timeout:?}");
        }
        let attempt = tokio::time::timeout(
            Duration::from_millis(200),
            tokio::net::TcpStream::connect((host, port)),
        );
        match attempt.await {
            Ok(Ok(_)) => break,
            Ok(Err(_)) | Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

async fn wait_for_nats_ready(
    url: &str,
    node: cheetah_signal_types::NodeId,
    resolver: Arc<dyn DeviceOwnerResolver>,
    timeout: Duration,
) -> Result<NatsBus, cheetah_message_api::BusError> {
    let started = std::time::Instant::now();
    loop {
        match NatsBus::connect_insecure(
            url,
            node,
            resolver.clone(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .await
        {
            Ok(bus) => return Ok(bus),
            Err(e) if started.elapsed() >= timeout => return Err(e),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

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

async fn wait_for_command(
    sub: &mut Box<dyn cheetah_message_api::Subscription<cheetah_message_api::CommandEnvelope>>,
    expected_operation_id: cheetah_signal_types::OperationId,
    timeout: Duration,
) -> Option<cheetah_message_api::Delivery<cheetah_message_api::CommandEnvelope>> {
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() >= timeout {
            return None;
        }
        let delivery = tokio::time::timeout(Duration::from_millis(500), sub.next()).await;
        match delivery {
            Ok(Ok(Some(d))) => {
                if let Ok(cmd) = decode_command(&d.envelope)
                    && cmd.operation_id() == expected_operation_id
                {
                    return Some(d);
                }
                let _ = d.ack.ack().await;
            }
            Ok(Ok(None)) => return None,
            Ok(Err(_)) | Err(_) => continue,
        }
    }
}

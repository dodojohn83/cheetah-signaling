//! Transactional outbox relay integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::PtzDirection;
use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
use cheetah_domain::{CommandPayload, Operation};
use cheetah_message_api::bus::RawEventBus;
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_application::OutboxRelay;
use cheetah_signal_types::{
    Clock, Event, IdGenerator, OwnerEpoch, ResourceId, ResourceKind, ResourceRef,
};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

async fn setup_outbox(
    path: &std::path::Path,
) -> (
    Arc<SqliteStorage>,
    Arc<InProcessMessageBus>,
    Arc<dyn Clock>,
    cheetah_signal_types::TenantId,
    cheetah_signal_types::DeviceId,
) {
    let storage = Arc::new(SqliteStorage::new(path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let bus = Arc::new(InProcessMessageBus::new(16, 16));
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    (storage, bus, clock, tenant_id, device_id)
}

#[tokio::test(flavor = "multi_thread")]
async fn outbox_relay_publishes_event_and_marks_published() {
    let file_id = InMemoryIdGenerator::new()
        .generate_message_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_outbox_relay_test_{file_id}.db"));
    let (storage, bus, clock, tenant_id, device_id) = setup_outbox(&path).await;

    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let context = cheetah_domain::in_memory::request_context(
        tenant_id,
        id_generator.as_ref(),
        clock.as_ref(),
    );
    let channel_id = id_generator.generate_channel_id();
    let target = ResourceRef {
        tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    };
    let (operation, domain_event) = Operation::new(
        id_generator.as_ref(),
        clock.as_ref(),
        &context,
        "outbox-test",
        device_id,
        target,
        CommandPayload::Ptz {
            channel_id,
            direction: PtzDirection::Stop,
            speed: 0.0,
        },
        None,
        OwnerEpoch::default(),
    )
    .unwrap();

    let event = Event::new(
        id_generator.as_ref(),
        clock.as_ref(),
        &context,
        tenant_id,
        ResourceRef {
            tenant_id,
            kind: ResourceKind::Operation,
            id: ResourceId::Operation(operation.operation_id()),
        },
        0,
        domain_event,
    );

    let mut uow = storage.begin().await.unwrap();
    uow.outbox().append(event).await.unwrap();
    uow.commit().await.unwrap();

    let mut subscription = bus.subscribe("", "").await.unwrap();

    let relay = OutboxRelay::new(storage.clone(), bus.clone(), clock.clone(), 10);
    let published = relay.run_once().await.unwrap();
    assert_eq!(published, 1, "outbox relay must publish one event");

    let received = tokio::time::timeout(std::time::Duration::from_secs(1), subscription.next())
        .await
        .unwrap()
        .unwrap()
        .expect("subscriber must receive the published event");
    assert!(!received.message_id.is_empty());

    let mut uow = storage.begin().await.unwrap();
    let pending = uow.outbox().pending(clock.now_wall(), 10).await.unwrap();
    uow.commit().await.unwrap();
    assert!(
        pending.is_empty(),
        "published event must be removed from pending outbox"
    );

    let _ = std::fs::remove_file(&path);
}

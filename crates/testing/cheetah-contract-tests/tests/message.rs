//! GB4-TST-003 message contracts: envelope round-trip + in-process bus
//! delivery semantics.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryIdGenerator, device_resource_ref, request_context,
};
use cheetah_domain::{
    Command, CommandBus, CommandPayload, DomainEvent, EventPublisher, MediaBinding, MediaPurpose,
    Operation,
};
use cheetah_message_api::{
    RawCommandBus, RawEventBus, decode_command, decode_event, encode_command, encode_event,
};
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_types::{Event, IdGenerator, OwnerEpoch, TenantId};

fn sample_command(clock: &InMemoryClock, id_generator: &InMemoryIdGenerator) -> Command {
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let context = request_context(tenant_id, id_generator, clock);
    let target = device_resource_ref(tenant_id, device_id);
    let payload = CommandPayload::StartLive {
        media_session_id: id_generator.generate_media_session_id(),
        channel_id: id_generator.generate_channel_id(),
        media_node_id: id_generator.generate_node_id(),
        purpose: MediaPurpose::Live,
    };
    let (operation, _) = Operation::new(
        id_generator,
        clock,
        &context,
        "idem-key",
        device_id,
        target,
        payload,
        None,
        OwnerEpoch(3),
    )
    .unwrap();
    operation.command().clone()
}

fn sample_event(
    clock: &InMemoryClock,
    id_generator: &InMemoryIdGenerator,
    tenant_id: TenantId,
) -> Event<DomainEvent> {
    let (_, domain_event) = MediaBinding::new(
        clock,
        id_generator.generate_media_binding_id(),
        id_generator.generate_media_session_id(),
        tenant_id,
        id_generator.generate_channel_id(),
        id_generator.generate_node_id(),
        OwnerEpoch::default(),
        cheetah_signal_types::MediaNodeInstanceEpoch(1),
    )
    .unwrap();
    let device_id = id_generator.generate_device_id();
    let context = request_context(tenant_id, id_generator, clock);
    let aggregate_ref = device_resource_ref(tenant_id, device_id);
    Event::new(
        id_generator,
        clock,
        &context,
        tenant_id,
        aggregate_ref,
        1,
        domain_event,
    )
}

#[test]
fn command_envelope_round_trips_with_metadata() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let command = sample_command(&clock, &id_generator);

    let envelope = encode_command(&command).unwrap();
    let meta = envelope.meta.as_ref().unwrap();
    assert_eq!(
        meta.message_id.as_ref().unwrap().value,
        command.message_id().to_string()
    );
    assert_eq!(
        meta.tenant_id.as_ref().unwrap().value,
        command.tenant_id().to_string()
    );
    assert_eq!(
        meta.correlation_id.as_ref().unwrap().value,
        command.correlation_id().to_string()
    );
    assert_eq!(meta.owner_epoch, command.expected_owner_epoch().0);
    assert_eq!(envelope.idempotency_key, command.idempotency_key());
    assert_eq!(envelope.operation_id, command.operation_id().to_string());

    let decoded = decode_command(&envelope).unwrap();
    assert_eq!(decoded, command);
}

#[test]
fn event_envelope_round_trips_with_aggregate_metadata() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let event = sample_event(&clock, &id_generator, tenant_id);

    let envelope = encode_event(&event).unwrap();
    assert_eq!(envelope.aggregate_sequence, event.aggregate_sequence);
    let meta = envelope.meta.as_ref().unwrap();
    assert_eq!(
        meta.message_id.as_ref().unwrap().value,
        event.event_id.to_string()
    );

    let decoded = decode_event(&envelope).unwrap();
    assert_eq!(decoded, event);
}

#[tokio::test]
async fn command_delivery_preserves_fifo_order() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let bus = InProcessMessageBus::new(16, 16);

    let mut sent_ids = Vec::new();
    for _ in 0..5 {
        let command = sample_command(&clock, &id_generator);
        sent_ids.push(command.message_id().to_string());
        CommandBus::send(&bus, &command).await.unwrap();
    }

    let mut subscription = RawCommandBus::subscribe(&bus, "", "group").await.unwrap();
    let mut received_ids = Vec::new();
    for _ in 0..5 {
        let delivery = subscription.next().await.unwrap().unwrap();
        received_ids.push(delivery.message_id);
    }
    assert_eq!(received_ids, sent_ids, "commands must arrive in send order");
}

/// At-least-once + idempotent consumption: the same command re-delivered twice
/// carries the same message id, so a consumer that dedupes by message id
/// processes it exactly once.
#[tokio::test]
async fn duplicate_command_deliveries_dedupe_by_message_id() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let bus = InProcessMessageBus::new(16, 16);

    let command = sample_command(&clock, &id_generator);
    CommandBus::send(&bus, &command).await.unwrap();
    CommandBus::send(&bus, &command).await.unwrap();

    let mut subscription = RawCommandBus::subscribe(&bus, "", "group").await.unwrap();
    let first = subscription.next().await.unwrap().unwrap();
    let second = subscription.next().await.unwrap().unwrap();
    assert_eq!(first.message_id, second.message_id);

    let mut seen = std::collections::BTreeSet::new();
    seen.insert(first.message_id);
    let inserted = seen.insert(second.message_id);
    assert!(!inserted, "second delivery is a duplicate and is deduped");
}

#[tokio::test]
async fn events_fan_out_to_all_subscribers() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let bus = InProcessMessageBus::new(16, 16);

    let mut sub_a = RawEventBus::subscribe(&bus, "", "group").await.unwrap();
    let mut sub_b = RawEventBus::subscribe(&bus, "", "group").await.unwrap();

    let event = sample_event(&clock, &id_generator, tenant_id);
    EventPublisher::publish(&bus, &event).await.unwrap();

    let a = sub_a.next().await.unwrap().unwrap();
    let b = sub_b.next().await.unwrap().unwrap();
    assert_eq!(a.message_id, event.event_id.to_string());
    assert_eq!(b.message_id, event.event_id.to_string());
}

#[tokio::test]
async fn publishing_without_subscribers_is_a_noop() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let bus = InProcessMessageBus::new(16, 16);
    let event = sample_event(&clock, &id_generator, tenant_id);
    // No subscribers: publish must succeed without blocking or erroring.
    EventPublisher::publish(&bus, &event).await.unwrap();
}

//! GB4-TST-003 media-port contract against the deterministic
//! `InMemoryMediaPort`. Control-plane only: reservations and typed node
//! commands, never RTP/RTCP or media payloads.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, InMemoryMediaPort};
use cheetah_domain::{
    CommandPayload, DomainError, MediaNodeCommand, MediaNodeCommandResult, MediaPort, MediaPurpose,
    MediaRequirements,
};
use cheetah_signal_types::{
    ChannelId, IdGenerator, MediaBindingId, MediaSessionId, NodeId, OwnerEpoch, PageRequest,
    TenantId,
};

struct Fixture {
    port: InMemoryMediaPort,
    ids: Arc<InMemoryIdGenerator>,
    clock: InMemoryClock,
}

fn fixture() -> Fixture {
    let ids = Arc::new(InMemoryIdGenerator::new());
    let port = InMemoryMediaPort::new(ids.clone() as Arc<dyn IdGenerator>);
    Fixture {
        port,
        ids,
        clock: InMemoryClock::new(),
    }
}

async fn reserve(
    fx: &Fixture,
    tenant_id: TenantId,
    binding_id: MediaBindingId,
) -> cheetah_domain::Result<cheetah_domain::MediaReservation> {
    fx.port
        .reserve_live(
            tenant_id,
            fx.ids.generate_device_id(),
            fx.ids.generate_channel_id(),
            fx.ids.generate_media_session_id(),
            binding_id,
            MediaPurpose::Live,
            &MediaRequirements::default(),
            &fx.clock,
        )
        .await
}

fn start_command(
    fx: &Fixture,
    tenant_id: TenantId,
    node_id: NodeId,
    media_session_id: MediaSessionId,
    channel_id: ChannelId,
) -> MediaNodeCommand {
    MediaNodeCommand {
        request_id: "req-1".to_string(),
        tenant_id,
        media_session_id,
        media_binding_id: fx.ids.generate_media_binding_id(),
        media_node_id: node_id,
        media_node_instance_epoch: cheetah_signal_types::MediaNodeInstanceEpoch(1),
        operation_id: fx.ids.generate_operation_id(),
        owner_epoch: OwnerEpoch::default(),
        source_node_id: fx.ids.generate_node_id(),
        deadline: None,
        idempotency_key: "idem-1".to_string(),
        contract_version: 1,
        payload: CommandPayload::StartLive {
            media_session_id,
            channel_id,
            media_node_id: node_id,
            purpose: MediaPurpose::Live,
        },
    }
}

#[tokio::test]
async fn reservation_is_deterministic_and_versioned() {
    let fx = fixture();
    let tenant_id = fx.ids.generate_tenant_id();
    let binding_id = fx.ids.generate_media_binding_id();
    let reservation = reserve(&fx, tenant_id, binding_id).await.unwrap();
    assert_eq!(reservation.contract_version, 1);
    assert!(!reservation.media_node_id.as_uuid().is_nil());
    assert!(reservation.media_node_instance_epoch.0 > 0);
}

#[tokio::test]
async fn duplicate_reservation_is_rejected() {
    let fx = fixture();
    let tenant_id = fx.ids.generate_tenant_id();
    let binding_id = fx.ids.generate_media_binding_id();
    reserve(&fx, tenant_id, binding_id).await.unwrap();
    let err = reserve(&fx, tenant_id, binding_id).await.unwrap_err();
    assert!(matches!(err, DomainError::Unavailable { .. }));
}

#[tokio::test]
async fn release_frees_the_binding_for_re_reservation() {
    let fx = fixture();
    let tenant_id = fx.ids.generate_tenant_id();
    let binding_id = fx.ids.generate_media_binding_id();
    reserve(&fx, tenant_id, binding_id).await.unwrap();
    fx.port
        .release(tenant_id, binding_id, &fx.clock)
        .await
        .unwrap();
    // Releasing is idempotent enough that a fresh reservation now succeeds.
    reserve(&fx, tenant_id, binding_id).await.unwrap();
}

#[tokio::test]
async fn tenant_isolation_keys_reservations_and_nodes() {
    let fx = fixture();
    let tenant_a = fx.ids.generate_tenant_id();
    let tenant_b = fx.ids.generate_tenant_id();
    let binding_id = fx.ids.generate_media_binding_id();
    // Same binding id under two tenants are distinct reservations.
    reserve(&fx, tenant_a, binding_id).await.unwrap();
    reserve(&fx, tenant_b, binding_id).await.unwrap();

    // A start command for tenant A must not appear in tenant B's node list.
    let node_id = fx.ids.generate_node_id();
    let media_session_id = fx.ids.generate_media_session_id();
    let channel_id = fx.ids.generate_channel_id();
    let result = fx
        .port
        .execute(
            start_command(&fx, tenant_a, node_id, media_session_id, channel_id),
            &fx.clock,
        )
        .await
        .unwrap();
    assert_eq!(result, MediaNodeCommandResult::Accepted);

    let nodes_a = fx.port.list_nodes(tenant_a, &fx.clock).await.unwrap();
    assert_eq!(nodes_a.len(), 1);
    let nodes_b = fx.port.list_nodes(tenant_b, &fx.clock).await.unwrap();
    assert!(nodes_b.is_empty(), "tenant B must not see tenant A nodes");
}

#[tokio::test]
async fn start_then_stop_command_lifecycle() {
    let fx = fixture();
    let tenant_id = fx.ids.generate_tenant_id();
    let node_id = fx.ids.generate_node_id();
    let media_session_id = fx.ids.generate_media_session_id();
    let channel_id = fx.ids.generate_channel_id();

    let start = fx
        .port
        .execute(
            start_command(&fx, tenant_id, node_id, media_session_id, channel_id),
            &fx.clock,
        )
        .await
        .unwrap();
    assert_eq!(start, MediaNodeCommandResult::Accepted);

    let sessions = fx
        .port
        .list_sessions(
            tenant_id,
            node_id,
            PageRequest {
                cursor: None,
                page_size: 10,
            },
            &fx.clock,
        )
        .await
        .unwrap();
    assert_eq!(sessions.items.len(), 1);
    assert_eq!(sessions.items[0].media_session_id, media_session_id);

    let mut stop = start_command(&fx, tenant_id, node_id, media_session_id, channel_id);
    stop.payload = CommandPayload::StopMediaSession { media_session_id };
    let stopped = fx.port.execute(stop, &fx.clock).await.unwrap();
    assert_eq!(stopped, MediaNodeCommandResult::Completed);

    let sessions = fx
        .port
        .list_sessions(
            tenant_id,
            node_id,
            PageRequest {
                cursor: None,
                page_size: 10,
            },
            &fx.clock,
        )
        .await
        .unwrap();
    assert!(sessions.items.is_empty(), "stop removes the node session");
}

#[tokio::test]
async fn device_command_is_not_dispatched_through_media_port() {
    let fx = fixture();
    let tenant_id = fx.ids.generate_tenant_id();
    let node_id = fx.ids.generate_node_id();
    let media_session_id = fx.ids.generate_media_session_id();
    let channel_id = fx.ids.generate_channel_id();
    let mut command = start_command(&fx, tenant_id, node_id, media_session_id, channel_id);
    command.payload = CommandPayload::Ptz {
        channel_id,
        direction: cheetah_domain::PtzDirection::Up,
        speed: 1.0,
    };
    let err = fx.port.execute(command, &fx.clock).await.unwrap_err();
    assert!(matches!(err, DomainError::InvalidArgument { .. }));
}

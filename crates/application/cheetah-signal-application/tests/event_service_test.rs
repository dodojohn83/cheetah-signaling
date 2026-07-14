//! Event service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{Clock, CommandPayload, PtzDirection, UnitOfWork};
use cheetah_signal_application::SubmitOperationRequest;
use cheetah_signal_types::{OwnerEpoch, ResourceId, ResourceKind, ResourceRef};
use common::*;

#[tokio::test(flavor = "current_thread")]
async fn event_service_publishes_outbox() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;

    let target = ResourceRef {
        tenant_id: ctx.tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device.device_id),
    };
    ctx.operation_service
        .submit_operation(
            &context,
            &mut ctx.uow,
            SubmitOperationRequest {
                device_id: device.device_id,
                target,
                payload: CommandPayload::Ptz {
                    channel_id: ctx.channel_id,
                    direction: PtzDirection::Right,
                    speed: 0.5,
                },
                idempotency_key: "ptz-1".to_string(),
                deadline: None,
                expected_owner_epoch: OwnerEpoch::default(),
            },
        )
        .await
        .unwrap();

    let published = ctx
        .event_service
        .publish_pending(
            ctx.uow.outbox(),
            &*ctx.event_publisher,
            ctx.clock.now_wall(),
            10,
        )
        .await
        .unwrap();
    assert!(published >= 1);

    ctx.uow.commit().await.unwrap();
    let committed = ctx.uow.committed();
    assert!(committed.outbox.iter().all(|e| e.published));
    assert_eq!(ctx.event_publisher.snapshot().len(), published);
}

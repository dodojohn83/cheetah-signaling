//! Operation service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{CommandPayload, OperationStatus, PtzDirection, UnitOfWork};
use cheetah_signal_application::SubmitOperationRequest;
use cheetah_signal_types::{OwnerEpoch, ResourceId, ResourceKind, ResourceRef};
use common::*;

#[tokio::test(flavor = "current_thread")]
async fn operation_service_idempotent_submission() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;

    let target = ResourceRef {
        tenant_id: ctx.tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device.device_id),
    };
    let request = SubmitOperationRequest {
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
    };

    let first = ctx
        .operation_service
        .submit_operation(&context, &mut ctx.uow, request.clone())
        .await
        .unwrap();

    for _ in 0..99 {
        let next = ctx
            .operation_service
            .submit_operation(&context, &mut ctx.uow, request.clone())
            .await
            .unwrap();
        assert_eq!(next.operation_id, first.operation_id);
    }

    let stored = ctx
        .uow
        .operation_repository()
        .get(ctx.tenant_id, first.operation_id)
        .await
        .unwrap();
    assert!(stored.is_some());
    assert_eq!(stored.unwrap().status(), OperationStatus::Pending);
}

#[tokio::test(flavor = "current_thread")]
async fn operation_service_different_keys_are_distinct() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;

    let target = ResourceRef {
        tenant_id: ctx.tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device.device_id),
    };
    let make_request = |key: &str| SubmitOperationRequest {
        device_id: device.device_id,
        target: target.clone(),
        payload: CommandPayload::Ptz {
            channel_id: ctx.channel_id,
            direction: PtzDirection::Right,
            speed: 0.5,
        },
        idempotency_key: key.to_string(),
        deadline: None,
        expected_owner_epoch: OwnerEpoch::default(),
    };

    let a = ctx
        .operation_service
        .submit_operation(&context, &mut ctx.uow, make_request("ptz-a"))
        .await
        .unwrap();
    let b = ctx
        .operation_service
        .submit_operation(&context, &mut ctx.uow, make_request("ptz-b"))
        .await
        .unwrap();
    assert_ne!(a.operation_id, b.operation_id);
}

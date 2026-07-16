//! Operation reconciler integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{CommandPayload, OperationStatus, PtzDirection, UnitOfWork};
use cheetah_signal_application::{OperationReconciler, SubmitOperationRequest};
use cheetah_signal_types::{
    Clock, Deadline, DurationMs, OwnerEpoch, ResourceId, ResourceKind, ResourceRef,
};
use common::*;

#[tokio::test(flavor = "current_thread")]
async fn operation_reconciler_times_out_expired_pending_operation() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;

    let now = ctx.clock.now_wall();
    let deadline = Deadline::from_now(now, DurationMs::from_millis(100)).unwrap();

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
        idempotency_key: "ptz-timeout".to_string(),
        deadline: Some(deadline),
        expected_owner_epoch: OwnerEpoch::default(),
    };

    let operation = ctx
        .operation_service
        .submit_operation(&context, &mut ctx.uow, request)
        .await
        .unwrap();

    ctx.clock.advance(DurationMs::from_millis(250));

    let reconciler =
        OperationReconciler::new(ctx.clock.clone(), ctx.id_generator.clone(), 100, 1000);
    let report = reconciler.reconcile(&context, &mut ctx.uow).await.unwrap();
    ctx.uow.commit().await.unwrap();

    assert_eq!(report.scanned, 1);
    assert_eq!(report.timed_out, 1);

    let stored = ctx
        .uow
        .operation_repository()
        .get(ctx.tenant_id, operation.operation_id)
        .await
        .unwrap()
        .expect("operation should exist");
    assert_eq!(stored.status(), OperationStatus::TimedOut);
    assert_eq!(stored.error().unwrap().code(), "expired_before_dispatch");
    assert!(stored.deadline().unwrap().is_elapsed(ctx.clock.now_wall()));
}

#[tokio::test(flavor = "current_thread")]
async fn operation_reconciler_times_out_expired_running_operation() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;

    let now = ctx.clock.now_wall();
    let deadline = Deadline::from_now(now, DurationMs::from_millis(100)).unwrap();

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
        idempotency_key: "ptz-running-timeout".to_string(),
        deadline: Some(deadline),
        expected_owner_epoch: OwnerEpoch::default(),
    };

    let operation = ctx
        .operation_service
        .submit_operation(&context, &mut ctx.uow, request)
        .await
        .unwrap();

    {
        let mut op = ctx
            .uow
            .operation_repository()
            .get(ctx.tenant_id, operation.operation_id)
            .await
            .unwrap()
            .expect("operation should exist");
        op.start(ctx.clock.as_ref()).unwrap();
        ctx.uow.operation_repository().save(&op).await.unwrap();
    }

    ctx.clock.advance(DurationMs::from_millis(250));

    let reconciler =
        OperationReconciler::new(ctx.clock.clone(), ctx.id_generator.clone(), 100, 1000);
    let report = reconciler.reconcile(&context, &mut ctx.uow).await.unwrap();
    ctx.uow.commit().await.unwrap();

    assert_eq!(report.scanned, 1);
    assert_eq!(report.timed_out, 1);

    let stored = ctx
        .uow
        .operation_repository()
        .get(ctx.tenant_id, operation.operation_id)
        .await
        .unwrap()
        .expect("operation should exist");
    assert_eq!(stored.status(), OperationStatus::TimedOut);
    assert_eq!(stored.error().unwrap().code(), "timeout");
}

#[tokio::test(flavor = "current_thread")]
async fn operation_reconciler_respects_max_timeouts_per_run() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;

    let now = ctx.clock.now_wall();
    let deadline = Deadline::from_now(now, DurationMs::from_millis(10)).unwrap();

    let make_request = |key: &str| SubmitOperationRequest {
        device_id: device.device_id,
        target: ResourceRef {
            tenant_id: ctx.tenant_id,
            kind: ResourceKind::Device,
            id: ResourceId::Device(device.device_id),
        },
        payload: CommandPayload::Ptz {
            channel_id: ctx.channel_id,
            direction: PtzDirection::Right,
            speed: 0.5,
        },
        idempotency_key: key.to_string(),
        deadline: Some(deadline),
        expected_owner_epoch: OwnerEpoch::default(),
    };

    for i in 0..3 {
        ctx.operation_service
            .submit_operation(&context, &mut ctx.uow, make_request(&format!("ptz-{i}")))
            .await
            .unwrap();
    }

    ctx.clock.advance(DurationMs::from_millis(50));

    let reconciler = OperationReconciler::new(ctx.clock.clone(), ctx.id_generator.clone(), 10, 2);
    let report = reconciler.reconcile(&context, &mut ctx.uow).await.unwrap();

    assert_eq!(report.scanned, 2);
    assert_eq!(report.timed_out, 2);
}

#[tokio::test(flavor = "current_thread")]
async fn operation_reconciler_ignores_not_yet_expired_deadline() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;

    let now = ctx.clock.now_wall();
    let deadline = Deadline::from_now(now, DurationMs::from_millis(10_000)).unwrap();

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
        idempotency_key: "ptz-future".to_string(),
        deadline: Some(deadline),
        expected_owner_epoch: OwnerEpoch::default(),
    };

    ctx.operation_service
        .submit_operation(&context, &mut ctx.uow, request)
        .await
        .unwrap();

    let reconciler =
        OperationReconciler::new(ctx.clock.clone(), ctx.id_generator.clone(), 100, 1000);
    let report = reconciler.reconcile(&context, &mut ctx.uow).await.unwrap();

    assert_eq!(report.scanned, 1);
    assert_eq!(report.timed_out, 0);
}

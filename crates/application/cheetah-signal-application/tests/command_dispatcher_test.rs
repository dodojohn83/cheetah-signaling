//! Command dispatcher integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use std::sync::Arc;

use cheetah_domain::{
    CommandBus, CommandPayload, DomainError, OperationStatus, OwnerInfo, PtzDirection,
};
use cheetah_signal_application::SubmitOperationRequest;
use cheetah_signal_types::{IdGenerator, OwnerEpoch, ResourceId, ResourceKind, ResourceRef};
use common::*;

struct FailingCommandBus;

#[async_trait::async_trait]
impl CommandBus for FailingCommandBus {
    async fn send(&self, _command: &cheetah_domain::Command) -> Result<(), DomainError> {
        Err(DomainError::unavailable("command bus unavailable"))
    }
}

async fn submit_ptz_operation(ctx: &mut TestContext) -> cheetah_signal_application::OperationDto {
    let context = request_context(ctx);
    let device = register_device_and_channel(ctx).await;
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
        .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn command_dispatcher_sends_command_to_owner() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let operation = submit_ptz_operation(&mut ctx).await;

    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        operation.device_id,
        OwnerInfo {
            owner_node_id: ctx.id_generator.generate_node_id(),
            owner_epoch: OwnerEpoch::default(),
        },
    );

    let dispatched = ctx
        .command_dispatcher
        .dispatch(
            &context,
            &mut ctx.uow,
            ctx.tenant_id,
            operation.operation_id,
        )
        .await
        .unwrap();
    assert_eq!(dispatched.status, OperationStatus::Running);
    assert_eq!(ctx.command_bus.snapshot().len(), 1);

    let redispatched = ctx
        .command_dispatcher
        .dispatch(
            &context,
            &mut ctx.uow,
            ctx.tenant_id,
            operation.operation_id,
        )
        .await
        .unwrap();
    assert_eq!(redispatched.status, OperationStatus::Running);
    assert_eq!(ctx.command_bus.snapshot().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn command_dispatcher_fails_when_no_owner() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let operation = submit_ptz_operation(&mut ctx).await;

    let dispatched = ctx
        .command_dispatcher
        .dispatch(
            &context,
            &mut ctx.uow,
            ctx.tenant_id,
            operation.operation_id,
        )
        .await
        .unwrap();
    assert_eq!(dispatched.status, OperationStatus::Failed);
    assert!(dispatched.result.as_ref().unwrap().error_code().is_some());
    assert_eq!(ctx.command_bus.snapshot().len(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn command_dispatcher_fails_on_stale_owner_epoch() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let operation = submit_ptz_operation(&mut ctx).await;

    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        operation.device_id,
        OwnerInfo {
            owner_node_id: ctx.id_generator.generate_node_id(),
            owner_epoch: OwnerEpoch(7),
        },
    );

    let dispatched = ctx
        .command_dispatcher
        .dispatch(
            &context,
            &mut ctx.uow,
            ctx.tenant_id,
            operation.operation_id,
        )
        .await
        .unwrap();
    assert_eq!(dispatched.status, OperationStatus::Failed);
    let code = dispatched.result.unwrap().error_code().unwrap().to_string();
    assert_eq!(code, "STALE_OWNER");
}

#[tokio::test(flavor = "current_thread")]
async fn command_dispatcher_fails_when_command_bus_rejects() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let operation = submit_ptz_operation(&mut ctx).await;

    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        operation.device_id,
        OwnerInfo {
            owner_node_id: ctx.id_generator.generate_node_id(),
            owner_epoch: OwnerEpoch::default(),
        },
    );

    let failing_bus: Arc<dyn CommandBus> = Arc::new(FailingCommandBus);
    let dispatcher = cheetah_signal_application::CommandDispatcher::new(
        ctx.clock.clone(),
        ctx.id_generator.clone(),
        ctx.owner_resolver.clone(),
        failing_bus,
    );

    let dispatched = dispatcher
        .dispatch(
            &context,
            &mut ctx.uow,
            ctx.tenant_id,
            operation.operation_id,
        )
        .await
        .unwrap();
    assert_eq!(dispatched.status, OperationStatus::Failed);
    let code = dispatched.result.unwrap().error_code().unwrap().to_string();
    assert!(code.contains("COMMAND_BUS"));
}

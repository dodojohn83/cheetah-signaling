//! Outbox reconciler integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{Connectivity, DeviceLifecycle, DomainEvent, UnitOfWork};
use cheetah_signal_application::OutboxReconciler;
use cheetah_signal_types::{Clock, DurationMs, Event, ResourceId, ResourceKind, ResourceRef};
use common::*;

#[tokio::test(flavor = "current_thread")]
async fn outbox_reconciler_dead_letters_old_entry() {
    let mut ctx = setup();
    let context = request_context(&ctx);

    let event = Event::new(
        &*ctx.id_generator,
        &*ctx.clock,
        &context,
        ctx.tenant_id,
        ResourceRef {
            tenant_id: ctx.tenant_id,
            kind: ResourceKind::Device,
            id: ResourceId::Device(ctx.device_id),
        },
        1,
        DomainEvent::DeviceOnlineChanged {
            tenant_id: ctx.tenant_id,
            device_id: ctx.device_id,
            connectivity: Connectivity::Online,
            lifecycle: DeviceLifecycle::Active,
            reason: None,
        },
    );
    ctx.uow.outbox().append(event).await.unwrap();

    ctx.clock.advance(DurationMs::from_millis(1001));

    let reconciler = OutboxReconciler::new(ctx.clock.clone(), DurationMs::from_millis(1000), 100);
    let report = reconciler.reconcile(&mut ctx.uow).await.unwrap();

    assert_eq!(report.scanned, 1);
    assert_eq!(report.dead_lettered, 1);

    ctx.uow.commit().await.unwrap();
    let pending = ctx
        .uow
        .outbox()
        .pending(ctx.clock.now_wall(), 100)
        .await
        .unwrap();
    assert!(pending.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn outbox_reconciler_leaves_recent_entry() {
    let mut ctx = setup();
    let context = request_context(&ctx);

    let event = Event::new(
        &*ctx.id_generator,
        &*ctx.clock,
        &context,
        ctx.tenant_id,
        ResourceRef {
            tenant_id: ctx.tenant_id,
            kind: ResourceKind::Device,
            id: ResourceId::Device(ctx.device_id),
        },
        1,
        DomainEvent::DeviceOnlineChanged {
            tenant_id: ctx.tenant_id,
            device_id: ctx.device_id,
            connectivity: Connectivity::Online,
            lifecycle: DeviceLifecycle::Active,
            reason: None,
        },
    );
    ctx.uow.outbox().append(event).await.unwrap();

    ctx.clock.advance(DurationMs::from_millis(500));

    let reconciler = OutboxReconciler::new(ctx.clock.clone(), DurationMs::from_millis(1000), 100);
    let report = reconciler.reconcile(&mut ctx.uow).await.unwrap();

    assert_eq!(report.scanned, 1);
    assert_eq!(report.dead_lettered, 0);
}

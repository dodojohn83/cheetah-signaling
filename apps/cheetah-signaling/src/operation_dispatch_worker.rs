//! Event-driven worker that dispatches submitted operations to the command bus.
//!
//! This worker subscribes to domain events and reacts to `OperationSubmitted`
//! by loading the pending operation and routing its command through the
//! `CommandDispatcher`. It makes the operation lifecycle event-driven and
//! resilient to process restarts: the outbox relay replays `OperationSubmitted`
//! events after a crash, and the dispatcher is idempotent because it checks the
//! current operation status before sending.

use cheetah_domain::DomainEvent;
use cheetah_message_api::{EventEnvelope, Subscription, decode_event};
use cheetah_signal_application::CommandDispatcher;
use cheetah_signal_types::{Event, MessageId, NodeId, Principal, PrincipalKind, RequestContext};
use cheetah_storage_api::Storage;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Spawns a worker that listens for `OperationSubmitted` events and dispatches
/// the corresponding operation commands.
pub fn spawn(
    command_dispatcher: CommandDispatcher,
    storage: Arc<dyn Storage>,
    mut subscription: Box<dyn Subscription<EventEnvelope>>,
    node_id: NodeId,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!(%node_id, "operation dispatch worker started");

        loop {
            let delivery = tokio::select! {
                _ = cancel.cancelled() => {
                    info!("operation dispatch worker cancelled");
                    return;
                }
                result = subscription.next() => match result {
                    Ok(Some(d)) => d,
                    Ok(None) => {
                        info!("operation dispatch event subscription closed");
                        return;
                    }
                    Err(e) => {
                        warn!(error = %e, "operation dispatch worker event subscription error");
                        return;
                    }
                }
            };

            let event: Event<DomainEvent> = match decode_event(&delivery.envelope) {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "failed to decode domain event; acknowledging");
                    let _ = delivery.ack.ack().await;
                    continue;
                }
            };

            let DomainEvent::OperationSubmitted {
                operation_id,
                tenant_id,
                ..
            } = event.payload
            else {
                let _ = delivery.ack.ack().await;
                continue;
            };

            let mut uow = match storage.begin().await {
                Ok(u) => u,
                Err(e) => {
                    warn!(error = %e, %tenant_id, %operation_id, "failed to begin unit of work for operation dispatch; message will be redelivered");
                    let _ = delivery.ack.nak(Some("storage begin failed")).await;
                    continue;
                }
            };

            let context = RequestContext {
                tenant_id,
                principal: Principal {
                    id: format!("node-{node_id}-operation-dispatcher"),
                    kind: PrincipalKind::Service,
                    scopes: vec!["system".to_string()],
                },
                message_id: MessageId::generate(),
                correlation_id: event.correlation_id,
                traceparent: None,
                tracestate: None,
                deadline: None,
                node_id: Some(node_id),
                source_ip: None,
            };

            match command_dispatcher
                .dispatch(&context, uow.as_mut(), tenant_id, operation_id)
                .await
            {
                Ok(_) => {
                    let _ = delivery.ack.ack().await;
                }
                Err(e) => {
                    warn!(error = %e, %tenant_id, %operation_id, "operation dispatch failed; message will be redelivered");
                    let _ = delivery.ack.nak(Some("dispatch failed")).await;
                }
            }
        }
    })
}

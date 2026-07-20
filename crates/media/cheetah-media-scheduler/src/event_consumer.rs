//! gRPC media event stream consumer.
//!
//! The consumer subscribes to every active media node, de-duplicates events by
//! `(tenant_id, event_id)` through the inbox table, and applies session-level
//! callbacks through the domain `MediaEventHandler`. Per-node cursors are
//! persisted in the same unit of work as the processed-message record so a
//! crash replay resumes safely after the last committed event.

use crate::config::MediaEventConsumerConfig;
use crate::error::SchedulerError;
use crate::mapper::map_media_event_to_callback;
use crate::registry::MediaNodeRegistry;
use cheetah_domain::{
    MediaEventHandler, MediaNode, ProcessedMessageRecord, ProcessedMessageStatus, UnitOfWork,
};
use cheetah_media_client::MediaControlClient;
use cheetah_signal_contracts::cheetah::media::v1::{MediaEvent, SubscribeRequest};
use cheetah_signal_types::{
    Clock, CorrelationId, DurationMs, MessageId, NodeId, OperationId, Principal, PrincipalKind,
    RequestContext, TenantId,
};
use cheetah_storage_api::Storage;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Trigger for out-of-band reconciliation when a sequence gap is detected.
#[async_trait::async_trait]
pub trait ReconciliationHandler: Send + Sync {
    /// Called when the consumer detects a missing sequence number for a node.
    async fn reconcile(
        &self,
        node_id: NodeId,
        tenant_id: TenantId,
        expected_sequence: u64,
        actual_sequence: u64,
    );
}

/// No-op reconciler that logs the gap and relies on the scheduled reconciler.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopReconciliationHandler;

#[async_trait::async_trait]
impl ReconciliationHandler for NoopReconciliationHandler {
    async fn reconcile(
        &self,
        node_id: NodeId,
        tenant_id: TenantId,
        expected_sequence: u64,
        actual_sequence: u64,
    ) {
        tracing::warn!(
            %node_id,
            %tenant_id,
            expected_sequence,
            actual_sequence,
            "media event sequence gap detected; reconciliation required"
        );
    }
}

/// Tracks the last processed sequence for each media node.
#[derive(Clone, Debug, Default)]
struct CursorState {
    sequences: Arc<Mutex<std::collections::BTreeMap<NodeId, u64>>>,
    running: Arc<Mutex<BTreeSet<NodeId>>>,
}

impl CursorState {
    fn last_sequence(&self, node_id: NodeId) -> u64 {
        self.sequences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&node_id)
            .copied()
            .unwrap_or(0)
    }

    fn update_sequence(&self, node_id: NodeId, sequence: u64) {
        let mut map = self.sequences.lock().unwrap_or_else(|p| p.into_inner());
        if map.get(&node_id).is_none_or(|s| *s < sequence) {
            map.insert(node_id, sequence);
        }
    }

    fn mark_running(&self, node_id: NodeId) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(node_id)
    }

    fn remove_running(&self, node_id: NodeId) {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&node_id);
    }
}

/// A gRPC consumer that applies media-node events to the control plane.
#[derive(Clone)]
pub struct MediaEventConsumer {
    node_registry: Arc<dyn MediaNodeRegistry>,
    stream_client: MediaControlClient,
    event_handler: Arc<dyn MediaEventHandler>,
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    source_node_id: NodeId,
    config: MediaEventConsumerConfig,
    reconciler: Arc<dyn ReconciliationHandler>,
    cursors: CursorState,
    permits: Arc<tokio::sync::Semaphore>,
}

impl std::fmt::Debug for MediaEventConsumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaEventConsumer")
            .field("source_node_id", &self.source_node_id)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl MediaEventConsumer {
    /// Creates a new consumer.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node_registry: Arc<dyn MediaNodeRegistry>,
        stream_client: MediaControlClient,
        event_handler: Arc<dyn MediaEventHandler>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
        source_node_id: NodeId,
        config: MediaEventConsumerConfig,
        reconciler: Arc<dyn ReconciliationHandler>,
    ) -> Self {
        let permits = Arc::new(tokio::sync::Semaphore::new(
            config.max_concurrent_subscriptions,
        ));
        Self {
            node_registry,
            stream_client,
            event_handler,
            storage,
            clock,
            source_node_id,
            config,
            reconciler,
            cursors: CursorState::default(),
            permits,
        }
    }

    /// Runs the consumer until `cancel` is triggered.
    pub async fn run(self: Arc<Self>, cancel: CancellationToken) -> Result<(), SchedulerError> {
        let poll = Duration::from_millis(self.config.poll_interval_ms);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = sleep(poll) => {}
            }

            let nodes = self.node_registry.list_active(self.clock.as_ref()).await;

            for node in nodes {
                if !self.cursors.mark_running(node.node_id) {
                    continue;
                }

                let permit = self.permits.clone().acquire_owned().await.map_err(|_| {
                    SchedulerError::Backend("subscription semaphore closed".to_string())
                })?;

                let self_clone = Arc::clone(&self);
                let child = cancel.child_token();
                tokio::spawn(async move {
                    let _permit = permit;
                    let node_id = node.node_id;
                    if let Err(e) = self_clone.subscribe_node(node, child).await {
                        tracing::warn!(%node_id, "media event subscription ended: {e}");
                    }
                    self_clone.cursors.remove_running(node_id);
                });
            }
        }
        Ok(())
    }

    async fn subscribe_node(
        &self,
        node: MediaNode,
        cancel: CancellationToken,
    ) -> Result<(), SchedulerError> {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                result = self.consume_node(&node, cancel.child_token()) => {
                    if let Err(e) = result {
                        tracing::warn!(node_id = %node.node_id, "media event stream error: {e}");
                    }
                }
            }

            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = sleep(Duration::from_millis(self.config.reconnect_interval_ms)) => {}
            }
        }
    }

    async fn consume_node(
        &self,
        node: &MediaNode,
        cancel: CancellationToken,
    ) -> Result<(), SchedulerError> {
        let cursor = self.cursors.last_sequence(node.node_id).to_string();
        let request = build_subscribe_request(node, &cursor, &self.config, self.source_node_id);
        let endpoint = &node.control_endpoint;

        let mut stream = self
            .stream_client
            .subscribe(endpoint, request)
            .await
            .map_err(|e| SchedulerError::EventStream(format!("{e}")))?;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                msg = stream.message() => match msg {
                    Ok(None) => return Ok(()),
                    Ok(Some(event)) => {
                        if let Err(e) = self.process_event(event, node).await {
                            tracing::warn!(node_id = %node.node_id, "failed to process media event: {e}");
                        }
                    }
                    Err(e) => return Err(SchedulerError::EventStream(format!("{e}"))),
                },
            }
        }
    }

    async fn process_event(
        &self,
        event: MediaEvent,
        node: &MediaNode,
    ) -> Result<(), SchedulerError> {
        let event_id = event.event_id.clone();
        let sequence = event.sequence;

        let (tenant_id, callback) = match map_media_event_to_callback(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::info!(
                    %event_id,
                    sequence,
                    "media event mapping failed; treating as diagnostic: {e}"
                );
                self.cursors.update_sequence(node.node_id, sequence);
                return Ok(());
            }
        };

        if callback.media_node_id != node.node_id
            || callback.media_node_instance_epoch.0 != node.instance_epoch
        {
            tracing::info!(
                %event_id,
                %tenant_id,
                node_id = %node.node_id,
                "media event from old node instance; treating as diagnostic"
            );
            self.cursors.update_sequence(node.node_id, sequence);
            return Ok(());
        }

        self.check_sequence_gap(node, tenant_id, sequence).await;

        let message_id = message_id_for_event(tenant_id, &event_id);
        let mut uow = self
            .storage
            .begin()
            .await
            .map_err(|e| SchedulerError::Backend(format!("{e}")))?;

        let record = ProcessedMessageRecord {
            tenant_id,
            message_id,
            idempotency_key: Some(event_id.clone()),
            status: ProcessedMessageStatus::Pending,
            result_payload: None,
            processed_at: self.clock.now_wall(),
            expires_at: self
                .clock
                .now_wall()
                .checked_add(DurationMs::from_millis(self.config.record_ttl_ms as i64)),
        };

        let existing = uow
            .processed_message_repository()
            .get_or_insert(record)
            .await?;

        if existing.is_some() {
            self.update_cursor(&mut *uow, node, sequence).await?;
            uow.commit().await?;
            return Ok(());
        }

        let context = self.build_request_context(tenant_id, &event);
        match self
            .event_handler
            .handle_media_event(&context, &mut *uow, callback)
            .await
        {
            Ok(()) => {
                let payload = format!("{{\"sequence\":{sequence},\"status\":\"completed\"}}");
                uow.processed_message_repository()
                    .complete(
                        tenant_id,
                        message_id,
                        ProcessedMessageStatus::Completed,
                        Some(payload),
                        self.clock.now_wall(),
                    )
                    .await?;
                self.update_cursor(&mut *uow, node, sequence).await?;
                uow.commit().await?;
            }
            Err(e) => {
                let payload = format!("{{\"sequence\":{sequence},\"error\":\"{e}\"}}");
                uow.processed_message_repository()
                    .complete(
                        tenant_id,
                        message_id,
                        ProcessedMessageStatus::Failed,
                        Some(payload),
                        self.clock.now_wall(),
                    )
                    .await?;
                self.update_cursor(&mut *uow, node, sequence).await?;
                uow.commit().await?;
                return Err(SchedulerError::Domain(e));
            }
        }

        Ok(())
    }

    async fn check_sequence_gap(&self, node: &MediaNode, tenant_id: TenantId, sequence: u64) {
        let last = self.cursors.last_sequence(node.node_id);
        if last != 0 && sequence != last.saturating_add(1) {
            self.reconciler
                .reconcile(node.node_id, tenant_id, last.saturating_add(1), sequence)
                .await;
        }
        self.cursors.update_sequence(node.node_id, sequence);
    }

    async fn update_cursor(
        &self,
        uow: &mut dyn UnitOfWork,
        node: &MediaNode,
        sequence: u64,
    ) -> Result<(), SchedulerError> {
        let tenant_id = TenantId::default();
        let message_id = message_id_for_node(node.node_id);
        let now = self.clock.now_wall();
        let record = ProcessedMessageRecord {
            tenant_id,
            message_id,
            idempotency_key: Some(format!("cursor:{}", node.node_id)),
            status: ProcessedMessageStatus::Pending,
            result_payload: None,
            processed_at: now,
            expires_at: now.checked_add(DurationMs::from_millis(self.config.record_ttl_ms as i64)),
        };

        uow.processed_message_repository()
            .get_or_insert(record)
            .await?;

        let payload = format!("{{\"sequence\":{sequence}}}");
        uow.processed_message_repository()
            .complete(
                tenant_id,
                message_id,
                ProcessedMessageStatus::Completed,
                Some(payload),
                now,
            )
            .await?;

        Ok(())
    }

    fn build_request_context(&self, tenant_id: TenantId, event: &MediaEvent) -> RequestContext {
        let correlation_id = if event.correlation_id.is_empty() {
            CorrelationId::generate()
        } else {
            std::str::FromStr::from_str(&event.correlation_id)
                .unwrap_or_else(|_| CorrelationId::generate())
        };

        RequestContext {
            tenant_id,
            principal: Principal {
                id: self.source_node_id.to_string(),
                kind: PrincipalKind::Service,
                scopes: vec!["media_event".to_string()],
            },
            message_id: MessageId::generate(),
            correlation_id,
            traceparent: if event.traceparent.is_empty() {
                None
            } else {
                Some(event.traceparent.clone())
            },
            tracestate: if event.tracestate.is_empty() {
                None
            } else {
                Some(event.tracestate.clone())
            },
            deadline: None,
            node_id: Some(self.source_node_id),
            source_ip: None,
        }
    }
}

fn build_subscribe_request(
    node: &MediaNode,
    cursor: &str,
    config: &MediaEventConsumerConfig,
    source_node_id: NodeId,
) -> SubscribeRequest {
    use cheetah_signal_contracts::cheetah::media::v1::MediaMutationContext;

    let request_message_id = MessageId::generate().to_string();
    let context = MediaMutationContext {
        tenant_id: String::new(),
        request_id: request_message_id.clone(),
        correlation_id: CorrelationId::generate().to_string(),
        message_id: request_message_id,
        idempotency_key: format!("subscribe-{}-{}", node.node_id, MessageId::generate()),
        deadline: None,
        source_signaling_node_id: source_node_id.to_string(),
        owner_epoch: 0,
        target_media_node_id: node.node_id.to_string(),
        target_media_node_instance_epoch: node.instance_epoch,
        operation_id: OperationId::generate().to_string(),
        operation_step_id: "subscribe".to_string(),
        media_session_id: None,
        media_binding_id: None,
        contract_version: node.contract_version as u64,
        traceparent: None,
        tracestate: None,
    };

    SubscribeRequest {
        context: Some(context),
        media_session_ids: Vec::new(),
        resume_cursor: cursor.to_string(),
        max_batch_size: config.max_batch_size,
        filter: None,
    }
}

fn message_id_for_event(tenant_id: TenantId, event_id: &str) -> MessageId {
    let namespace = tenant_id.as_uuid();
    MessageId::from_uuid(Uuid::new_v5(&namespace, event_id.as_bytes()))
}

fn message_id_for_node(node_id: NodeId) -> MessageId {
    let name = format!("cursor:{}", node_id);
    MessageId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn message_id_for_event_is_deterministic() {
        let tenant = TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap();
        let a = message_id_for_event(tenant, "event-1");
        let b = message_id_for_event(tenant, "event-1");
        assert_eq!(a, b);
        let c = message_id_for_event(tenant, "event-2");
        assert_ne!(a, c);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn build_subscribe_request_uses_empty_tenant_for_all_tenants() {
        let node = MediaNode {
            node_id: NodeId::from_str("22222222-2222-2222-2222-222222222222").unwrap(),
            instance_epoch: 5,
            contract_version: 2,
            ..Default::default()
        };
        let config = MediaEventConsumerConfig::test();
        let source = NodeId::from_str("33333333-3333-3333-3333-333333333333").unwrap();

        let request = build_subscribe_request(&node, "cursor-1", &config, source);
        assert!(request.context.is_some());
        let ctx = request.context.unwrap_or_default();
        assert!(ctx.tenant_id.is_empty());
        assert_eq!(ctx.target_media_node_id, node.node_id.to_string());
        assert_eq!(ctx.target_media_node_instance_epoch, 5);
        assert_eq!(ctx.contract_version, 2);
        assert_eq!(request.resume_cursor, "cursor-1");
    }
}

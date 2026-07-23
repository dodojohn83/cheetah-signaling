//! gRPC media event stream consumer.
//!
//! The consumer subscribes to every active media node, de-duplicates events by
//! `(tenant_id, event_id)` through the inbox table, and applies session-level
//! callbacks through the domain `MediaEventHandler`. Per-node cursors are
//! persisted in the same unit of work as the processed-message record so a
//! crash replay resumes safely after the last committed event.

use crate::config::MediaEventConsumerConfig;
use crate::error::SchedulerError;
use crate::event_consumer_support::*;
use crate::metrics::MediaMetrics;
use crate::registry::MediaNodeRegistry;
use cheetah_domain::{
    DomainError, MediaClient, MediaEventHandler, MediaNode, MediaNodeCallback, MediaNodeEvent,
    MediaSubscriptionRequest, NodeStatus, ProcessedMessageRecord, ProcessedMessageStatus,
    UnitOfWork,
};
use cheetah_signal_types::{
    Clock, CorrelationId, DurationMs, MediaNodeInstanceEpoch, MessageId, NodeId, Principal,
    PrincipalKind, RequestContext, TenantId, UtcTimestamp,
};
use cheetah_storage_api::Storage;
use futures::StreamExt;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

pub use crate::event_consumer_support::{NoopReconciliationHandler, ReconciliationHandler};

/// Computes the expiry timestamp for a processed-message record.
///
/// If `ttl_ms` is so large that `now + ttl` overflows `OffsetDateTime`, the
/// result is clamped to the latest representable UTC timestamp so the record
/// remains evictable instead of being stored with a permanent `NULL` expiry.
fn ttl_expires_at(now: UtcTimestamp, ttl_ms: u64) -> UtcTimestamp {
    let ttl = i64::try_from(ttl_ms).unwrap_or(i64::MAX);
    now.checked_add(DurationMs::from_millis(ttl))
        .unwrap_or_else(|| {
            UtcTimestamp::from_offset(time::OffsetDateTime::new_in_offset(
                time::Date::from_calendar_date(9999, time::Month::December, 31)
                    .unwrap_or(time::Date::MAX),
                time::Time::from_hms(23, 59, 59).unwrap_or(time::Time::MIDNIGHT),
                time::UtcOffset::UTC,
            ))
        })
}
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(1);
const MAX_SLEEP: Duration = Duration::from_secs(24 * 60 * 60);

fn bounded_duration_from_ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
        .min(MAX_SLEEP)
        .max(MIN_POLL_INTERVAL)
}
/// A gRPC consumer that applies media-node events to the control plane.
#[derive(Clone)]
pub struct MediaEventConsumer {
    node_registry: Arc<dyn MediaNodeRegistry>,
    stream_client: Arc<dyn MediaClient>,
    event_handler: Arc<dyn MediaEventHandler>,
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    source_node_id: NodeId,
    config: MediaEventConsumerConfig,
    reconciler: Arc<dyn ReconciliationHandler>,
    cursors: CursorState,
    permits: Arc<tokio::sync::Semaphore>,
    subscriptions: SubscriptionState,
    diagnostic_log_limiter: DiagnosticLogLimiter,
    metrics: Arc<MediaMetrics>,
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
        stream_client: Arc<dyn MediaClient>,
        event_handler: Arc<dyn MediaEventHandler>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
        source_node_id: NodeId,
        config: MediaEventConsumerConfig,
        reconciler: Arc<dyn ReconciliationHandler>,
        metrics: Arc<MediaMetrics>,
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
            subscriptions: SubscriptionState::default(),
            diagnostic_log_limiter: DiagnosticLogLimiter::default(),
            metrics,
        }
    }

    /// Runs the consumer until `cancel` is triggered.
    pub async fn run(self: Arc<Self>, cancel: CancellationToken) -> Result<(), SchedulerError> {
        let poll = bounded_duration_from_ms(self.config.poll_interval_ms);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = sleep(poll) => {}
            }

            let nodes = self.node_registry.list_active(self.clock.as_ref()).await;
            let active: BTreeSet<NodeId> = nodes
                .into_iter()
                .filter(|n| n.status != NodeStatus::Left)
                .map(|n| n.node_id)
                .collect();
            Arc::clone(&self)
                .reconcile_subscriptions(active, &cancel)
                .await?;
        }
        Ok(())
    }

    async fn reconcile_subscriptions(
        self: Arc<Self>,
        active: BTreeSet<NodeId>,
        cancel: &CancellationToken,
    ) -> Result<(), SchedulerError> {
        let current = self.subscriptions.snapshot();
        let mut to_stop: Vec<NodeId> = Vec::new();
        let mut to_start: Vec<NodeId> = Vec::new();

        for id in current.keys() {
            if !active.contains(id) {
                to_stop.push(*id);
            }
        }

        for id in active {
            if !current.contains_key(&id) {
                to_start.push(id);
            }
        }

        for id in to_stop {
            self.subscriptions.cancel(id);
        }

        let nodes_to_start = to_start.len();
        for (index, node_id) in to_start.into_iter().enumerate() {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let permit = match self.permits.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(tokio::sync::TryAcquireError::NoPermits) => {
                    // No subscription slot is free right now; the main loop will retry
                    // on the next poll as other nodes stop or get cancelled.
                    let remaining = nodes_to_start.saturating_sub(index + 1);
                    tracing::warn!(
                        "media event consumer has reached max_concurrent_subscriptions; deferring subscription for remaining {} node(s)",
                        remaining
                    );
                    break;
                }
                Err(tokio::sync::TryAcquireError::Closed) => {
                    return Err(SchedulerError::Backend(
                        "subscription semaphore closed".to_string(),
                    ));
                }
            };

            let generation = self.subscriptions.next_generation();
            let token = cancel.child_token();
            let self_clone = Arc::clone(&self);
            let task_token = token.clone();

            self.subscriptions.insert(
                node_id,
                Subscription {
                    token: token.clone(),
                    generation,
                },
            );

            tokio::spawn(async move {
                let _permit = permit;
                if let Err(e) = self_clone
                    .subscribe_node(node_id, task_token, generation)
                    .await
                {
                    tracing::warn!(%node_id, "media event subscription ended: {e}");
                }
                self_clone
                    .subscriptions
                    .remove_if_generation(node_id, generation);
            });
        }

        Ok(())
    }

    async fn subscribe_node(
        &self,
        node_id: NodeId,
        cancel: CancellationToken,
        generation: u64,
    ) -> Result<(), SchedulerError> {
        let mut delay_ms = self.config.reconnect_interval_ms;
        loop {
            let result = tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                r = self.consume_node(node_id, cancel.child_token(), generation) => r,
            };
            if let Err(e) = result {
                tracing::warn!(%node_id, "media event stream error: {e}");
                delay_ms = delay_ms
                    .saturating_mul(2)
                    .min(self.config.max_reconnect_interval_ms);
            } else {
                delay_ms = self.config.reconnect_interval_ms;
            }

            // Add up to 25% jitter to avoid synchronized reconnect storms.
            let jitter = delay_ms / 4;
            let sleep_ms = delay_ms.saturating_add(fastrand::u64(0..=jitter));

            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = sleep(bounded_duration_from_ms(sleep_ms)) => {}
            }
        }
    }

    async fn consume_node(
        &self,
        node_id: NodeId,
        cancel: CancellationToken,
        _generation: u64,
    ) -> Result<(), SchedulerError> {
        self.metrics.record_event_reconnect();

        let Some(node) = self.node_registry.get(node_id, self.clock.as_ref()).await else {
            return Err(SchedulerError::NodeNotFound(node_id.to_string()));
        };

        self.load_cursor(&node).await?;

        let last = self.cursors.last_sequence(node.node_id);
        let cursor = if last == 0 {
            String::new()
        } else {
            last.to_string()
        };
        let request = MediaSubscriptionRequest {
            media_node_id: node.node_id,
            media_node_instance_epoch: MediaNodeInstanceEpoch(node.instance_epoch),
            source_node_id: self.source_node_id,
            resume_cursor: cursor,
            max_batch_size: self.config.max_batch_size,
            contract_version: node.contract_version,
            tenant_id: None,
        };
        let endpoint = &node.control_endpoint;

        let mut stream = self
            .stream_client
            .subscribe(endpoint, request)
            .await
            .map_err(|e| SchedulerError::EventStream(format!("{e}")))?;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                msg = stream.next() => match msg {
                    None => return Ok(()),
                    Some(Ok(event)) => {
                        if let Err(e) = self.process_event(event, &node).await {
                            tracing::warn!(node_id = %node.node_id, "failed to process media event: {e}");
                        }
                    }
                    Some(Err(e)) => return Err(SchedulerError::EventStream(format!("{e}"))),
                },
            }
        }
    }

    async fn load_cursor(&self, node: &MediaNode) -> Result<(), SchedulerError> {
        let mut uow = self
            .storage
            .begin()
            .await
            .map_err(|e| SchedulerError::Backend(format!("{e}")))?;
        let record = uow
            .processed_message_repository()
            .find(TenantId::default(), message_id_for_node(node.node_id))
            .await?;
        uow.commit().await?;

        if let Some(sequence) = record
            .and_then(|r| r.result_payload)
            .and_then(|p| parse_cursor_payload(&p))
        {
            self.cursors.update_sequence(node.node_id, sequence);
        }

        Ok(())
    }

    async fn process_event(
        &self,
        event: MediaNodeEvent,
        node: &MediaNode,
    ) -> Result<(), SchedulerError> {
        let event_id = event.event_id.clone();
        let sequence = event.sequence;
        let tenant_id = event.tenant_id;

        if event_id.is_empty() || tenant_id.as_uuid().is_nil() {
            if self.diagnostic_log_limiter.check(
                node.node_id,
                self.config.max_diagnostic_logs_per_second,
                self.clock.now_wall(),
            ) {
                tracing::info!(
                    %node.node_id,
                    sequence,
                    "media event missing event_id or tenant_id; treating as diagnostic"
                );
            }
            self.detect_sequence_gap(node, tenant_id, sequence).await;
            self.cursors.update_sequence(node.node_id, sequence);
            return Ok(());
        }

        if let Some(occurred) = event.occurred_at {
            let lag =
                (self.clock.now_wall().as_offset() - occurred.as_offset()).whole_milliseconds();
            if lag >= 0 {
                let lag_ms = u64::try_from(lag).unwrap_or(u64::MAX);
                self.metrics.record_event_lag_ms(lag_ms);
            }
        }

        let callback = match event.callback {
            Some(cb) => cb,
            None => {
                if self.diagnostic_log_limiter.check(
                    node.node_id,
                    self.config.max_diagnostic_logs_per_second,
                    self.clock.now_wall(),
                ) {
                    tracing::info!(
                        %event_id,
                        %tenant_id,
                        sequence,
                        "media event callback could not be parsed; treating as diagnostic"
                    );
                }
                self.detect_sequence_gap(node, tenant_id, sequence).await;
                self.cursors.update_sequence(node.node_id, sequence);
                return Ok(());
            }
        };

        if callback.media_node_id != node.node_id
            || callback.media_node_instance_epoch.0 != node.instance_epoch
        {
            if self.diagnostic_log_limiter.check(
                node.node_id,
                self.config.max_diagnostic_logs_per_second,
                self.clock.now_wall(),
            ) {
                tracing::info!(
                    %event_id,
                    %tenant_id,
                    node_id = %node.node_id,
                    "media event from old node instance; treating as diagnostic"
                );
            }
            self.detect_sequence_gap(node, tenant_id, sequence).await;
            self.cursors.update_sequence(node.node_id, sequence);
            return Ok(());
        }

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
            expires_at: Some(ttl_expires_at(
                self.clock.now_wall(),
                self.config.record_ttl_ms,
            )),
        };

        let existing = uow
            .processed_message_repository()
            .get_or_insert(record)
            .await?;

        if existing.is_some() {
            self.update_cursor(&mut *uow, node, sequence).await?;
            uow.commit().await?;
            self.detect_sequence_gap(node, tenant_id, sequence).await;
            self.cursors.update_sequence(node.node_id, sequence);
            return Ok(());
        }

        // Pre-validate ownership before delegating to the event handler. This
        // ensures a misbehaving media node cannot drive state transitions for
        // tenants or bindings it does not own.
        if let Err(e) = self
            .validate_callback(&mut *uow, tenant_id, &callback, node)
            .await
        {
            let payload = serde_json::to_string(&serde_json::json!({
                "sequence": sequence,
                "error": e.to_string(),
            }))
            .map_err(|e| SchedulerError::Backend(format!("{e}")))?;
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
            self.detect_sequence_gap(node, tenant_id, sequence).await;
            self.cursors.update_sequence(node.node_id, sequence);
            return Ok(());
        }

        let context = self.build_request_context(
            tenant_id,
            event.traceparent.as_deref(),
            event.tracestate.as_deref(),
            &event.correlation_id,
        );
        let result = self
            .event_handler
            .handle_media_event(&context, &mut *uow, callback)
            .await;

        match result {
            Ok(()) => {
                let payload = serde_json::to_string(&serde_json::json!({
                    "sequence": sequence,
                    "status": "completed",
                }))
                .map_err(|e| SchedulerError::Backend(format!("{e}")))?;
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
                self.detect_sequence_gap(node, tenant_id, sequence).await;
                self.cursors.update_sequence(node.node_id, sequence);
                Ok(())
            }
            Err(e) => {
                // Discard any partial outbox/state writes from the failed handler.
                // The processed-message failure marker and cursor are recorded in
                // a fresh unit of work so the domain transaction stays atomic.
                uow.rollback()
                    .await
                    .map_err(|e| SchedulerError::Backend(format!("{e}")))?;
                drop(uow);

                let payload = serde_json::to_string(&serde_json::json!({
                    "sequence": sequence,
                    "error": e.to_string(),
                }))
                .map_err(|e| SchedulerError::Backend(format!("{e}")))?;

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
                    expires_at: Some(ttl_expires_at(
                        self.clock.now_wall(),
                        self.config.record_ttl_ms,
                    )),
                };
                uow.processed_message_repository()
                    .get_or_insert(record)
                    .await?;
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
                self.detect_sequence_gap(node, tenant_id, sequence).await;
                self.cursors.update_sequence(node.node_id, sequence);
                Err(SchedulerError::Domain(e))
            }
        }
    }

    /// Verifies that the callback references a binding and session that really
    /// belong to the connected node instance and the claimed tenant. This is a
    /// consumer-side guard so that a compromised media node cannot drive state
    /// transitions for arbitrary tenants before the event handler is invoked.
    async fn validate_callback(
        &self,
        uow: &mut dyn UnitOfWork,
        tenant_id: TenantId,
        callback: &MediaNodeCallback,
        node: &MediaNode,
    ) -> Result<(), DomainError> {
        if callback.media_node_id != node.node_id {
            return Err(DomainError::invalid_argument("media node id mismatch"));
        }
        if callback.media_node_instance_epoch.0 != node.instance_epoch {
            return Err(DomainError::invalid_argument(
                "media node instance epoch mismatch",
            ));
        }

        let binding = uow
            .media_binding_repository()
            .get(tenant_id, callback.media_binding_id)
            .await?
            .ok_or_else(|| {
                DomainError::not_found("media binding", callback.media_binding_id.to_string())
            })?;
        if binding.media_node_id() != callback.media_node_id {
            return Err(DomainError::invalid_argument("media binding node mismatch"));
        }
        if binding.media_node_instance_epoch() != callback.media_node_instance_epoch {
            return Err(DomainError::invalid_argument(
                "media binding instance epoch mismatch",
            ));
        }
        if binding.owner_epoch() != callback.owner_epoch {
            return Err(DomainError::invalid_argument("owner epoch mismatch"));
        }
        if binding.revision().0 != callback.binding_revision.0 {
            return Err(DomainError::ConcurrentModification {
                expected: callback.binding_revision.0,
                found: binding.revision().0,
            });
        }

        let session = uow
            .media_session_repository()
            .get(tenant_id, callback.media_session_id)
            .await?
            .ok_or_else(|| {
                DomainError::not_found("media session", callback.media_session_id.to_string())
            })?;
        if session.revision().0 != callback.session_revision.0 {
            return Err(DomainError::ConcurrentModification {
                expected: callback.session_revision.0,
                found: session.revision().0,
            });
        }

        Ok(())
    }

    async fn detect_sequence_gap(&self, node: &MediaNode, tenant_id: TenantId, sequence: u64) {
        let last = self.cursors.last_sequence(node.node_id);
        if last != 0 && sequence > last.saturating_add(1) {
            self.metrics.record_event_gap();
            self.reconciler
                .reconcile(node.node_id, tenant_id, last.saturating_add(1), sequence)
                .await;
        }
    }

    async fn update_cursor(
        &self,
        uow: &mut dyn UnitOfWork,
        node: &MediaNode,
        sequence: u64,
    ) -> Result<(), SchedulerError> {
        let tenant_id = TenantId::default();
        let message_id = message_id_for_node(node.node_id);

        // Avoid regressing the persisted cursor if an older/duplicate event is
        // re-delivered out of order.
        if let Some(record) = uow
            .processed_message_repository()
            .find(tenant_id, message_id)
            .await?
            && let Some(existing) = record
                .result_payload
                .as_deref()
                .and_then(parse_cursor_payload)
            && sequence <= existing
        {
            return Ok(());
        }

        let now = self.clock.now_wall();
        let record = ProcessedMessageRecord {
            tenant_id,
            message_id,
            idempotency_key: Some(format!("cursor:{}", node.node_id)),
            status: ProcessedMessageStatus::Pending,
            result_payload: None,
            processed_at: now,
            expires_at: Some(ttl_expires_at(now, self.config.record_ttl_ms)),
        };

        uow.processed_message_repository()
            .get_or_insert(record)
            .await?;

        let payload = serde_json::to_string(&serde_json::json!({ "sequence": sequence }))
            .map_err(|e| SchedulerError::Backend(format!("{e}")))?;
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

    fn build_request_context(
        &self,
        tenant_id: TenantId,
        traceparent: Option<&str>,
        tracestate: Option<&str>,
        correlation_id: &str,
    ) -> RequestContext {
        let correlation_id = if correlation_id.is_empty() {
            CorrelationId::generate()
        } else {
            std::str::FromStr::from_str(correlation_id)
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
            traceparent: traceparent.map(|s| s.to_string()),
            tracestate: tracestate.map(|s| s.to_string()),
            deadline: None,
            node_id: Some(self.source_node_id),
            source_ip: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn ttl_expires_at_clamps_overflow_to_representable_max() {
        let now = UtcTimestamp::from_offset(OffsetDateTime::now_utc());

        // A normal TTL produces a finite future timestamp.
        let short = ttl_expires_at(now, 60_000);
        assert!(short.as_offset() > now.as_offset());

        // `u64::MAX` milliseconds overflows `OffsetDateTime`; it must not
        // return `None` (which would store a permanent `NULL` expiry) and
        // must not panic.
        let far = ttl_expires_at(now, u64::MAX);
        assert!(far.as_offset() > now.as_offset());
        assert!(far.as_offset().year() >= 9999);
    }

    #[test]
    fn bounded_duration_clamps_huge_ms_to_max() {
        assert_eq!(bounded_duration_from_ms(u64::MAX), MAX_SLEEP);
    }

    #[test]
    fn bounded_duration_clamps_zero_to_min() {
        assert_eq!(bounded_duration_from_ms(0), MIN_POLL_INTERVAL);
    }

    #[test]
    fn bounded_duration_preserves_reasonable_value() {
        let d = Duration::from_millis(5_000);
        assert_eq!(bounded_duration_from_ms(5_000), d);
    }
}

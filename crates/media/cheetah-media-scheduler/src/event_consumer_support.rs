//! Supporting state and utilities for the media event consumer.

use crate::config::MediaEventConsumerConfig;
use cheetah_domain::MediaNode;
use cheetah_signal_contracts::cheetah::media::v1::SubscribeRequest;
use cheetah_signal_types::{CorrelationId, MessageId, NodeId, OperationId, TenantId, UtcTimestamp};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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
pub(crate) struct CursorState {
    sequences: Arc<Mutex<BTreeMap<NodeId, u64>>>,
}

impl CursorState {
    pub(crate) fn last_sequence(&self, node_id: NodeId) -> u64 {
        self.sequences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&node_id)
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn update_sequence(&self, node_id: NodeId, sequence: u64) {
        let mut map = self.sequences.lock().unwrap_or_else(|p| p.into_inner());
        if map.get(&node_id).is_none_or(|s| *s < sequence) {
            map.insert(node_id, sequence);
        }
    }
}

/// Simple per-node token bucket for diagnostic log rate limiting.
#[derive(Clone, Debug, Default)]
pub(crate) struct DiagnosticLogLimiter {
    buckets: Arc<Mutex<BTreeMap<NodeId, TokenBucket>>>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TokenBucket {
    tokens: f64,
    last_update: UtcTimestamp,
}

impl DiagnosticLogLimiter {
    pub(crate) fn check(&self, node_id: NodeId, max_per_second: u32, now: UtcTimestamp) -> bool {
        let max = f64::from(max_per_second.max(1));
        let mut buckets = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        let bucket = buckets.entry(node_id).or_insert_with(|| TokenBucket {
            tokens: max,
            last_update: now,
        });

        let elapsed = ((now.as_offset().unix_timestamp_nanos()
            - bucket.last_update.as_offset().unix_timestamp_nanos()) as f64)
            / 1_000_000_000.0;
        bucket.tokens = (bucket.tokens + elapsed.max(0.0) * max).min(max);
        bucket.last_update = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Subscription {
    pub(crate) token: CancellationToken,
    pub(crate) generation: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SubscriptionState {
    inner: Arc<Mutex<BTreeMap<NodeId, Subscription>>>,
    next_generation: Arc<AtomicU64>,
}

impl SubscriptionState {
    pub(crate) fn next_generation(&self) -> u64 {
        self.next_generation.fetch_add(1, Ordering::SeqCst)
    }

    pub(crate) fn insert(&self, node_id: NodeId, subscription: Subscription) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(node_id, subscription);
    }

    pub(crate) fn remove_if_generation(&self, node_id: NodeId, generation: u64) {
        let mut subs = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if subs
            .get(&node_id)
            .is_some_and(|s| s.generation == generation)
        {
            subs.remove(&node_id);
        }
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<NodeId, Subscription> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub(crate) fn cancel(&self, node_id: NodeId) {
        let mut subs = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(sub) = subs.remove(&node_id) {
            sub.token.cancel();
        }
    }
}
pub(crate) fn build_subscribe_request(
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

pub(crate) fn message_id_for_event(tenant_id: TenantId, event_id: &str) -> MessageId {
    let namespace = tenant_id.as_uuid();
    MessageId::from_uuid(Uuid::new_v5(&namespace, event_id.as_bytes()))
}

pub(crate) fn message_id_for_node(node_id: NodeId) -> MessageId {
    let name = format!("cursor:{}", node_id);
    MessageId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()))
}

pub(crate) fn parse_cursor_payload(payload: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct CursorPayload {
        sequence: u64,
    }

    serde_json::from_str::<CursorPayload>(payload)
        .ok()
        .map(|cursor| cursor.sequence)
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

    #[test]
    fn parse_cursor_payload_extracts_sequence() {
        assert_eq!(parse_cursor_payload("{\"sequence\":42}"), Some(42));
        assert_eq!(parse_cursor_payload("not-json"), None);
    }
}

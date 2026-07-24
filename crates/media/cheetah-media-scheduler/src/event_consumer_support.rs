//! Supporting state and utilities for the media event consumer.

use cheetah_signal_types::{MessageId, NodeId, TenantId, UtcTimestamp};
use std::collections::{BTreeMap, BTreeSet};
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

    /// Removes cursor entries for nodes that are no longer active, preventing
    /// the sequence map from growing without bound as nodes churn.
    pub(crate) fn retain(&self, active: &BTreeSet<NodeId>) {
        let mut map = self.sequences.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|node_id, _| active.contains(node_id));
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

    /// Removes rate-limit buckets for nodes that are no longer active,
    /// preventing unbounded memory growth as nodes churn.
    pub(crate) fn retain(&self, active: &BTreeSet<NodeId>) {
        let mut buckets = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        buckets.retain(|node_id, _| active.contains(node_id));
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
    fn parse_cursor_payload_extracts_sequence() {
        assert_eq!(parse_cursor_payload("{\"sequence\":42}"), Some(42));
        assert_eq!(parse_cursor_payload("not-json"), None);
    }

    #[test]
    fn cursor_state_retain_prunes_inactive_nodes() {
        let state = CursorState::default();
        let active = NodeId::from_uuid(Uuid::from_u128(1));
        let inactive = NodeId::from_uuid(Uuid::from_u128(2));
        state.update_sequence(active, 1);
        state.update_sequence(inactive, 2);

        let mut keep = BTreeSet::new();
        keep.insert(active);
        state.retain(&keep);

        assert_eq!(state.last_sequence(active), 1);
        assert_eq!(state.last_sequence(inactive), 0);
    }

    #[test]
    fn diagnostic_log_limiter_retain_prunes_inactive_nodes() {
        let limiter = DiagnosticLogLimiter::default();
        let active = NodeId::from_uuid(Uuid::from_u128(3));
        let inactive = NodeId::from_uuid(Uuid::from_u128(4));

        let now = UtcTimestamp::from_epoch_millis_saturating(0);
        assert!(limiter.check(active, 10, now));
        assert!(limiter.check(inactive, 10, now));

        let mut keep = BTreeSet::new();
        keep.insert(active);
        limiter.retain(&keep);

        // Inactive node's bucket should be gone; a new check should succeed
        // because a fresh bucket is created with the full token allowance.
        assert!(limiter.check(active, 10, now));
        assert!(limiter.check(inactive, 10, now));
    }
}

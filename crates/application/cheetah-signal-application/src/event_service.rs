//! Event publishing service.

use cheetah_domain::{EventPublisher, Outbox, OutboxEntry};
use cheetah_signal_types::{DurationMs, UtcTimestamp, hash::stable_hash_u64};
use tracing::warn;

const MAX_ATTEMPTS: u32 = 10;
const BASE_BACKOFF_MS: i64 = 100;

/// Publishes pending outbox events, retrying transient failures.
#[derive(Clone, Debug, Default)]
pub struct EventService;

impl EventService {
    /// Creates a new event service.
    pub fn new() -> Self {
        Self
    }

    /// Reads up to `limit` pending outbox events as of `now`.
    pub async fn read_pending(
        &self,
        outbox: &mut dyn Outbox,
        now: UtcTimestamp,
        limit: usize,
    ) -> crate::Result<Vec<OutboxEntry>> {
        Ok(outbox.pending(now, limit).await?)
    }

    /// Publishes each event to `publisher` and returns the per-event result.
    ///
    /// This call performs external I/O and must not be invoked while a database
    /// transaction is held open.
    pub async fn publish_events(
        &self,
        publisher: &dyn EventPublisher,
        entries: &[OutboxEntry],
    ) -> Vec<crate::Result<()>> {
        let mut results = Vec::with_capacity(entries.len());
        for entry in entries {
            results.push(
                publisher
                    .publish(&entry.event)
                    .await
                    .map_err(crate::SignalError::from),
            );
        }
        results
    }

    /// Records the outcome of a publish attempt for each entry.
    ///
    /// Successful results mark the event as published. Failures increment the
    /// attempt count and schedule a retry with exponential backoff and
    /// deterministic jitter derived from the event ID.
    pub async fn record_results(
        &self,
        outbox: &mut dyn Outbox,
        now: UtcTimestamp,
        entries: &[OutboxEntry],
        results: &[crate::Result<()>],
    ) -> crate::Result<usize> {
        let mut published = 0;
        for (entry, result) in entries.iter().zip(results.iter()) {
            match result {
                Ok(()) => {
                    outbox.mark_published(entry.event.event_id).await?;
                    published += 1;
                }
                Err(e) => {
                    let attempts = entry.attempts + 1;
                    let failed = attempts >= MAX_ATTEMPTS;
                    let (error, next_attempt_at) = if failed {
                        (Some(e.to_string()), None)
                    } else {
                        let backoff_ms = BASE_BACKOFF_MS * (1i64 << attempts.min(20));
                        let jitter_range = backoff_ms / 4;
                        let jitter_ms = if jitter_range > 0 {
                            let hash = stable_hash_u64(&entry.event.event_id);
                            (hash % jitter_range as u64) as i64
                        } else {
                            0
                        };
                        let total_backoff = backoff_ms + jitter_ms;
                        let next = now
                            .checked_add(DurationMs::from_millis(total_backoff))
                            .unwrap_or(now);
                        (Some(e.to_string()), Some(next))
                    };
                    warn!(
                        event_id = %entry.event.event_id.as_uuid(),
                        attempts,
                        failed,
                        "event publish failed"
                    );
                    outbox
                        .mark_failed(
                            entry.event.event_id,
                            attempts,
                            failed,
                            error,
                            next_attempt_at,
                        )
                        .await?;
                }
            }
        }
        Ok(published)
    }

    /// Publishes pending outbox events in a single call.
    ///
    /// The caller is responsible for ensuring that `outbox` is not backed by
    /// an open SQL transaction while external I/O is performed. For production
    /// relays, use `read_pending`, `publish_events`, and `record_results`
    /// outside and inside separate transactions instead.
    pub async fn publish_pending(
        &self,
        outbox: &mut dyn Outbox,
        publisher: &dyn EventPublisher,
        now: UtcTimestamp,
        limit: usize,
    ) -> crate::Result<usize> {
        let entries = self.read_pending(outbox, now, limit).await?;
        let results = self.publish_events(publisher, &entries).await;
        self.record_results(outbox, now, &entries, &results).await
    }
}

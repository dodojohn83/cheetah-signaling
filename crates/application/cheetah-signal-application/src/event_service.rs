//! Event publishing service.

use cheetah_domain::{EventPublisher, Outbox};
use cheetah_signal_types::{DurationMs, UtcTimestamp};
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

    /// Publishes up to `limit` pending outbox events as of `now`.
    ///
    /// Each event is published once. On success it is marked published. On
    /// failure it is marked failed with an incrementing attempt count and a
    /// scheduled retry time. After `MAX_ATTEMPTS` the event enters the permanent
    /// failure state.
    pub async fn publish_pending(
        &self,
        outbox: &mut dyn Outbox,
        publisher: &dyn EventPublisher,
        now: UtcTimestamp,
        limit: usize,
    ) -> crate::Result<usize> {
        let entries = outbox.pending(now, limit).await?;
        let mut published = 0;

        for entry in &entries {
            let attempts = entry.attempts + 1;
            match publisher.publish(&entry.event).await {
                Ok(()) => {
                    outbox.mark_published(entry.event.event_id).await?;
                    published += 1;
                }
                Err(e) => {
                    let failed = attempts >= MAX_ATTEMPTS;
                    let (error, next_attempt_at) = if failed {
                        (Some(e.to_string()), None)
                    } else {
                        let backoff_ms = BASE_BACKOFF_MS * (1i64 << attempts.min(20));
                        let next = now
                            .checked_add(DurationMs::from_millis(backoff_ms))
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
}

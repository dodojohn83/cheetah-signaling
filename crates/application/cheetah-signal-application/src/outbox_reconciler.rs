//! Outbox reconciler.
//!
//! Scans the transactional outbox for pending entries that have exceeded a
//! maximum age and marks them as permanently failed (dead-lettered). This
//! prevents a stalled or backlogged message bus from keeping incomplete
//! outbox records alive indefinitely.

use crate::dto::OutboxReconciliationReport;
use crate::event_service::{MAX_RETRY_BACKOFF, TOTAL_RETRY_WINDOW};
use cheetah_domain::{Clock, UnitOfWork};
use cheetah_signal_types::DurationMs;
use std::sync::Arc;
use tracing::warn;

const DEAD_LETTER_REASON: &str = "outbox entry exceeded maximum age and was dead-lettered";

/// Reconciles the outbox by dead-lettering entries that are too old.
#[derive(Clone)]
pub struct OutboxReconciler {
    clock: Arc<dyn Clock>,
    max_age: DurationMs,
    batch_size: usize,
}

impl OutboxReconciler {
    /// Creates a new outbox reconciler.
    ///
    /// `max_age` should be configured larger than the outbox relay's total
    /// cumulative retry window (`TOTAL_RETRY_WINDOW`) so the reconciler does not
    /// dead-letter events that the relay could still publish.
    pub fn new(clock: Arc<dyn Clock>, max_age: DurationMs, batch_size: usize) -> Self {
        Self {
            clock,
            max_age,
            batch_size: batch_size.max(1),
        }
    }

    /// Scans pending or retry-scheduled outbox entries and dead-letters those
    /// whose `occurred_at` is older than `max_age`.
    ///
    /// The caller is responsible for committing the unit of work.
    pub async fn reconcile(
        &self,
        uow: &mut dyn UnitOfWork,
    ) -> crate::Result<OutboxReconciliationReport> {
        let now = self.clock.now_wall();

        // Probe far enough into the future that an old entry cannot hide
        // behind a future `next_attempt_at`. The total retry window is the
        // maximum `next_attempt_at - occurred_at`, while `max_age +
        // MAX_RETRY_BACKOFF` covers a recent entry that has just been scheduled
        // for its next retry.
        let probe_offset = self
            .max_age
            .as_millis()
            .saturating_add(MAX_RETRY_BACKOFF.as_millis())
            .max(TOTAL_RETRY_WINDOW.as_millis());
        let probe_now = now
            .checked_add(DurationMs::from_millis(probe_offset))
            .unwrap_or(now);
        let entries = uow.outbox().pending(probe_now, self.batch_size).await?;

        let age_threshold = now.checked_sub(self.max_age);
        let mut report = OutboxReconciliationReport::default();

        for entry in entries {
            report.scanned += 1;

            if age_threshold.is_some_and(|threshold| entry.event.occurred_at <= threshold) {
                warn!(
                    event_id = %entry.event.event_id.as_uuid(),
                    tenant_id = %entry.event.tenant_id.as_uuid(),
                    "dead-lettering outbox entry older than max age"
                );
                let dead_lettered = uow
                    .outbox()
                    .mark_failed(
                        entry.event.event_id,
                        entry.attempts,
                        true,
                        Some(DEAD_LETTER_REASON.to_string()),
                        None,
                    )
                    .await?;
                if dead_lettered {
                    report.dead_lettered += 1;
                }
            }
        }

        Ok(report)
    }
}

impl std::fmt::Debug for OutboxReconciler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboxReconciler")
            .field("max_age", &self.max_age)
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

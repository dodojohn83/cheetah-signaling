//! Operation reconciler.
//!
//! Scans non-terminal operations and transitions those whose deadlines have
//! elapsed to the `TimedOut` terminal state. The caller is responsible for
//! committing the unit of work.

use crate::dto::OperationReconciliationReport;
use crate::operation_service::{operation_resource_ref, wrap_event};
use cheetah_domain::{OperationError, OperationStatus, UnitOfWork};
use cheetah_signal_types::{Clock, IdGenerator, MAX_PAGE_SIZE, PageRequest, RequestContext};
use std::sync::Arc;

/// Reconciles pending and running operations against their deadlines.
#[derive(Clone)]
pub struct OperationReconciler {
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    batch_size: u32,
    max_timeouts_per_run: u64,
}

impl OperationReconciler {
    /// Creates a new operation reconciler.
    ///
    /// `batch_size` is clamped to the range `[1, MAX_PAGE_SIZE]` so the
    /// configured value cannot cause `PageRequest::new` to fail at runtime.
    pub fn new(
        clock: Arc<dyn Clock>,
        id_generator: Arc<dyn IdGenerator>,
        batch_size: u32,
        max_timeouts_per_run: u64,
    ) -> Self {
        Self {
            clock,
            id_generator,
            batch_size: batch_size.clamp(1, MAX_PAGE_SIZE),
            max_timeouts_per_run,
        }
    }

    /// Reconciles pending and running operations for the tenant in `context`.
    ///
    /// Operations whose deadline has passed are transitioned to `TimedOut`
    /// and an outbox event is appended. The caller is responsible for
    /// committing the unit of work.
    pub async fn reconcile(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
    ) -> crate::Result<OperationReconciliationReport> {
        let tenant_id = context.tenant_id;
        let now = self.clock.now_wall();
        let mut report = OperationReconciliationReport::default();

        for status in [OperationStatus::Pending, OperationStatus::Running] {
            if report.timed_out >= self.max_timeouts_per_run {
                break;
            }

            let mut cursor: Option<String> = None;
            loop {
                if report.timed_out >= self.max_timeouts_per_run {
                    break;
                }

                let request = match cursor {
                    None => PageRequest::new(self.batch_size)?,
                    Some(c) => PageRequest::new(self.batch_size)?.with_cursor(c),
                };
                let page = uow
                    .operation_repository()
                    .list(tenant_id, None, Some(status.to_string()), None, request)
                    .await?;

                for mut operation in page.items {
                    if report.timed_out >= self.max_timeouts_per_run {
                        break;
                    }

                    report.scanned += 1;
                    if operation.is_terminal() || operation.status() != status {
                        continue;
                    }

                    if let Some(deadline) = operation.deadline()
                        && deadline.is_elapsed(now)
                    {
                        let event = if status == OperationStatus::Pending {
                            operation.expire(self.clock.as_ref())
                        } else {
                            operation.timeout(OperationError::timeout(), self.clock.as_ref())
                        }
                        .map_err(crate::SignalError::from)?;
                        uow.operation_repository().save(&operation).await?;
                        let event = wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            tenant_id,
                            operation_resource_ref(tenant_id, operation.operation_id()),
                            operation.revision().0,
                            event,
                        );
                        uow.outbox().append(event).await?;
                        report.timed_out += 1;
                    }
                }

                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }

        Ok(report)
    }
}

impl std::fmt::Debug for OperationReconciler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationReconciler")
            .field("batch_size", &self.batch_size)
            .field("max_timeouts_per_run", &self.max_timeouts_per_run)
            .finish_non_exhaustive()
    }
}

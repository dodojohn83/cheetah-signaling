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
    ///
    /// The reconciler round-robins between `Pending` and `Running` operations
    /// one at a time so that a sustained backlog of expired pending operations
    /// cannot starve expired running operations.
    pub async fn reconcile(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
    ) -> crate::Result<OperationReconciliationReport> {
        let tenant_id = context.tenant_id;
        let now = self.clock.now_wall();
        let mut report = OperationReconciliationReport::default();

        let mut cursors = [
            StatusCursor::new(OperationStatus::Pending, "pending"),
            StatusCursor::new(OperationStatus::Running, "running"),
        ];

        while report.timed_out < self.max_timeouts_per_run && cursors.iter().any(|c| !c.exhausted())
        {
            for cursor in &mut cursors {
                if report.timed_out >= self.max_timeouts_per_run {
                    break;
                }

                if let Some(mut operation) = cursor
                    .next_operation(uow, tenant_id, self.batch_size)
                    .await?
                {
                    report.scanned += 1;
                    if operation.is_terminal() || operation.status() != cursor.status {
                        continue;
                    }

                    if let Some(deadline) = operation.deadline()
                        && deadline.is_elapsed(now)
                    {
                        let event = if cursor.status == OperationStatus::Pending {
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
            }
        }

        Ok(report)
    }
}

struct StatusCursor {
    status: OperationStatus,
    filter: &'static str,
    cursor: Option<String>,
    buffer: Vec<cheetah_domain::Operation>,
    has_more: bool,
}

impl StatusCursor {
    fn new(status: OperationStatus, filter: &'static str) -> Self {
        Self {
            status,
            filter,
            cursor: None,
            buffer: Vec::new(),
            has_more: true,
        }
    }

    fn exhausted(&self) -> bool {
        !self.has_more && self.buffer.is_empty()
    }

    async fn next_operation(
        &mut self,
        uow: &mut dyn UnitOfWork,
        tenant_id: cheetah_signal_types::TenantId,
        batch_size: u32,
    ) -> crate::Result<Option<cheetah_domain::Operation>> {
        if self.buffer.is_empty() {
            if !self.has_more {
                return Ok(None);
            }

            let request = match self.cursor.take() {
                None => PageRequest::new(batch_size)?,
                Some(c) => PageRequest::new(batch_size)?.with_cursor(c),
            };
            let page = uow
                .operation_repository()
                .list(
                    tenant_id,
                    None,
                    Some(self.filter.to_string()),
                    None,
                    request,
                )
                .await?;

            let has_more = page.next_cursor.is_some();
            self.cursor = page.next_cursor;
            self.has_more = has_more;
            self.buffer = page.items;

            if self.buffer.is_empty() && !self.has_more {
                return Ok(None);
            }
        }

        Ok(self.buffer.pop())
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

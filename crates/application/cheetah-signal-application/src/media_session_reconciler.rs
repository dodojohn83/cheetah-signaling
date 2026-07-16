//! Reconciles `MediaSession` desired state and terminal `Operation` results.

use crate::media_reconciliation::{
    converge_active, fail_session_and_binding, release_session_and_binding,
};
use cheetah_domain::{MediaSession, MediaSessionDesiredState, UnitOfWork};
use cheetah_signal_types::{
    Clock, IdGenerator, MAX_PAGE_SIZE, MediaBindingId, PageRequest, RequestContext,
};
use std::sync::Arc;

/// Report produced by a `MediaSessionReconciler` pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaSessionReconciliationReport {
    /// Number of non-terminal sessions scanned.
    pub scanned: u64,
    /// Number of sessions released because their desired state is `Stopped`.
    pub released: u64,
    /// Number of sessions converged to `Active` from a successful operation.
    pub activated: u64,
    /// Number of sessions failed because their operation failed or is missing.
    pub failed: u64,
    /// Scheduler reservations that the caller should release after committing.
    pub reservations_to_release: Vec<MediaBindingId>,
}

/// Reconciles the logical desired state of media sessions and their owning
/// operations, driving session and binding state machines without talking to
/// media nodes.
#[derive(Clone)]
pub struct MediaSessionReconciler {
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    batch_size: u32,
}

impl std::fmt::Debug for MediaSessionReconciler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaSessionReconciler")
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

impl MediaSessionReconciler {
    /// Creates a new reconciler.
    pub fn new(clock: Arc<dyn Clock>, id_generator: Arc<dyn IdGenerator>, batch_size: u32) -> Self {
        Self {
            clock,
            id_generator,
            batch_size: batch_size.clamp(1, MAX_PAGE_SIZE),
        }
    }

    /// Scans non-terminal media sessions for this tenant and converges them to
    /// their desired state using the owning operation's terminal result.
    ///
    /// The caller is responsible for committing `uow` and releasing any
    /// `reservations_to_release` returned in the report.
    pub async fn reconcile(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
    ) -> crate::Result<MediaSessionReconciliationReport> {
        let tenant_id = context.tenant_id;
        let mut report = MediaSessionReconciliationReport::default();

        let mut cursor: Option<String> = None;
        loop {
            let request = match cursor {
                None => PageRequest::new(self.batch_size)?,
                Some(c) => PageRequest::new(self.batch_size)?.with_cursor(c),
            };
            let page = uow
                .media_session_repository()
                .list(tenant_id, None, None, None, None, request)
                .await?;

            for mut session in page.items {
                if let Some(mut binding) = uow
                    .media_binding_repository()
                    .get_by_media_session(tenant_id, session.media_session_id())
                    .await?
                {
                    if binding.is_terminal() {
                        continue;
                    }
                    report.scanned += 1;
                    let session_rev_before = session.revision().0;
                    let binding_rev_before = binding.revision().0;
                    self.reconcile_one(context, uow, &mut session, &mut binding, &mut report)
                        .await?;
                    if session.revision().0 != session_rev_before {
                        uow.media_session_repository().save(&session).await?;
                    }
                    if binding.revision().0 != binding_rev_before {
                        uow.media_binding_repository().save(&binding).await?;
                    }
                }
            }

            match page.next_cursor {
                None => break,
                Some(c) => cursor = Some(c),
            }
        }

        Ok(report)
    }

    async fn reconcile_one(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        session: &mut MediaSession,
        binding: &mut cheetah_domain::MediaBinding,
        report: &mut MediaSessionReconciliationReport,
    ) -> crate::Result<()> {
        // A terminal session may still hold a non-terminal binding (e.g. a
        // Reserved binding that was never activated, or a callback that stopped
        // the session without releasing the binding). Release it before any
        // other path can try to activate an already-terminal session.
        if session.is_terminal() {
            release_session_and_binding(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                uow,
                session,
                binding,
            )
            .await?;
            report.released += 1;
            report
                .reservations_to_release
                .push(binding.media_binding_id());
            return Ok(());
        }

        if session.desired_state() == MediaSessionDesiredState::Stopped {
            release_session_and_binding(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                uow,
                session,
                binding,
            )
            .await?;
            report.released += 1;
            report
                .reservations_to_release
                .push(binding.media_binding_id());
            return Ok(());
        }

        let operation = uow
            .operation_repository()
            .get(context.tenant_id, session.operation_id())
            .await?;
        let Some(operation) = operation else {
            fail_session_and_binding(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                uow,
                session,
                binding,
                "missing_operation",
                "owning operation not found",
            )
            .await?;
            report.failed += 1;
            report
                .reservations_to_release
                .push(binding.media_binding_id());
            return Ok(());
        };

        if !operation.is_terminal() {
            return Ok(());
        }

        match operation.result() {
            Some(result) if result.is_success() => {
                converge_active(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    uow,
                    session,
                    binding,
                )
                .await?;
                report.activated += 1;
            }
            Some(result) => {
                let code = result.error_code().unwrap_or("operation_failed");
                let message = result.error_message().unwrap_or("operation failed");
                fail_session_and_binding(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    uow,
                    session,
                    binding,
                    code,
                    message,
                )
                .await?;
                report.failed += 1;
                report
                    .reservations_to_release
                    .push(binding.media_binding_id());
            }
            None => {
                fail_session_and_binding(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    uow,
                    session,
                    binding,
                    "operation_terminal_no_result",
                    "operation ended without a result",
                )
                .await?;
                report.failed += 1;
                report
                    .reservations_to_release
                    .push(binding.media_binding_id());
            }
        }

        Ok(())
    }
}

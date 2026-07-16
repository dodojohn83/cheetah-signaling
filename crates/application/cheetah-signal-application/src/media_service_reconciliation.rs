//! Media session reconciliation with media node reports.

use crate::dto::ReconciliationReport;
use crate::media_binding_reconciler::MediaBindingReconciler;
use crate::media_service::MediaService;
use crate::media_session_reconciler::MediaSessionReconciler;
use cheetah_signal_types::RequestContext;

impl MediaService {
    /// Reconciles local media session and binding state with the sessions
    /// currently reported by each media node.
    ///
    /// This method delegates to `MediaSessionReconciler` (desired state and
    /// terminal operation results) and `MediaBindingReconciler` (database state
    /// against actual media-node resources). Both run in the same unit of work,
    /// which is committed once before scheduler reservations are released.
    pub async fn reconcile(
        &self,
        context: &RequestContext,
        uow: &mut dyn cheetah_domain::UnitOfWork,
    ) -> crate::Result<ReconciliationReport> {
        let session_reconciler =
            MediaSessionReconciler::new(self.clock.clone(), self.id_generator.clone(), 1000);
        let session_report = session_reconciler.reconcile(context, uow).await?;

        let binding_reconciler = MediaBindingReconciler::new(
            self.clock.clone(),
            self.id_generator.clone(),
            self.media_port.clone(),
            1000,
        );
        let binding_report = binding_reconciler.reconcile(context, uow).await?;

        uow.commit().await?;

        let mut reservations_to_release = session_report.reservations_to_release;
        reservations_to_release.extend(binding_report.reservations_to_release);
        for binding_id in reservations_to_release {
            if let Err(e) = self
                .media_port
                .release(context.tenant_id, binding_id, self.clock.as_ref())
                .await
            {
                tracing::warn!(
                    tenant_id = %context.tenant_id,
                    binding_id = %binding_id,
                    "failed to release scheduler reservation after reconciliation: {e}"
                );
            }
        }

        Ok(ReconciliationReport {
            nodes_scanned: binding_report.nodes_scanned,
            sessions_found: binding_report.sessions_found,
            missing_released: session_report.released,
            missing_failed: session_report.failed + binding_report.missing_failed,
            orphans_detected: binding_report.orphans_detected,
        })
    }
}

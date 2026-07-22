//! Supporting helpers for media session reconciliation.
//!
//! Split out from `media_service_reconciliation.rs` to keep the main module under
//! the repository's file-size limit.

use crate::dto::ReconciliationReport;
use crate::media_service::*;
use crate::media_service_helpers::*;
use cheetah_domain::{
    CommandPayload, DomainError, MediaBinding, MediaBindingError, MediaBindingState, MediaNode,
    MediaNodeCommand, MediaNodeHealth, MediaNodeSessionRef, MediaSession, MediaSessionState,
    NodeStatus,
};
use cheetah_signal_types::{
    Deadline, DeviceId, DurationMs, MediaBindingId, NodeId, OwnerEpoch, RequestContext, TenantId,
    UtcTimestamp,
};

impl MediaService {
    /// Advances an active `MediaSession` and its `MediaBinding` through any
    /// pending state transitions after a successful reconcile callback or start.
    pub(crate) async fn converge_active(
        &self,
        context: &RequestContext,
        uow: &mut dyn cheetah_domain::UnitOfWork,
        session: &mut MediaSession,
        binding: &mut MediaBinding,
    ) -> crate::Result<()> {
        match session.state() {
            MediaSessionState::Requested => {
                let ev = session.allocating(self.clock.as_ref())?;
                append_session_event(self, context, uow, session, ev).await?;
                uow.media_session_repository().save(session).await?;
                let ev = session.inviting(self.clock.as_ref())?;
                append_session_event(self, context, uow, session, ev).await?;
                uow.media_session_repository().save(session).await?;
                let ev = session.active(self.clock.as_ref())?;
                append_session_event(self, context, uow, session, ev).await?;
                uow.media_session_repository().save(session).await?;
            }
            MediaSessionState::Allocating => {
                let ev = session.inviting(self.clock.as_ref())?;
                append_session_event(self, context, uow, session, ev).await?;
                uow.media_session_repository().save(session).await?;
                let ev = session.active(self.clock.as_ref())?;
                append_session_event(self, context, uow, session, ev).await?;
                uow.media_session_repository().save(session).await?;
            }
            MediaSessionState::Inviting => {
                let ev = session.active(self.clock.as_ref())?;
                append_session_event(self, context, uow, session, ev).await?;
                uow.media_session_repository().save(session).await?;
            }
            MediaSessionState::Active => {}
            _ => {
                return Err(crate::SignalError::from(DomainError::invalid_transition(
                    "MediaSession",
                    format!("{:?}", session.state()),
                    "Active",
                )));
            }
        }

        if binding.state() == MediaBindingState::Reserved {
            let ev = binding.activate(self.clock.as_ref())?;
            append_binding_event(self, context, uow, binding, ev).await?;
            uow.media_binding_repository().save(binding).await?;
        } else if binding.state() == MediaBindingState::NeedsVerification {
            let ev = binding.verified(self.clock.as_ref())?;
            append_binding_event(self, context, uow, binding, ev).await?;
            uow.media_binding_repository().save(binding).await?;
        }

        Ok(())
    }

    /// Marks an already-active binding as needing verification instead of failing
    /// the session.
    ///
    /// Used when the media node's lease has expired or the node is unhealthy but
    /// has not been explicitly removed from the cluster, giving the node a
    /// chance to recover before the session is declared failed. Callers must
    /// only invoke this for sessions/bindings that are already `Active`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn mark_binding_needs_verification(
        &self,
        context: &RequestContext,
        uow: &mut dyn cheetah_domain::UnitOfWork,
        session: &mut MediaSession,
        binding: &mut MediaBinding,
        code: &str,
        message: &str,
        report: &mut ReconciliationReport,
    ) -> crate::Result<()> {
        if session.state() != MediaSessionState::Active
            || binding.state() != MediaBindingState::Active
        {
            return Ok(());
        }

        let ev = binding
            .needs_verification(MediaBindingError::new(code, message), self.clock.as_ref())?;
        append_binding_event(self, context, uow, binding, ev).await?;
        uow.media_binding_repository().save(binding).await?;

        report.needs_verification += 1;
        Ok(())
    }

    /// Sends a `StopMediaSession` command to a media node for a session that has
    /// no local signaling binding (orphan). This does not create a signaling
    /// operation or binding because the session is not tracked by this tenant.
    ///
    /// The owner epoch is resolved from the device owner resolver when the orphan
    /// reference includes a device id; otherwise a sentinel non-zero epoch is used
    /// because the real media scheduler rejects commands without an owner epoch.
    pub(crate) async fn stop_orphan_session(
        &self,
        tenant_id: TenantId,
        node_id: NodeId,
        node: &MediaNode,
        orphan: &MediaNodeSessionRef,
    ) -> crate::Result<()> {
        let owner_epoch = self
            .resolve_orphan_owner_epoch(tenant_id, orphan.device_id)
            .await;

        let now = self.clock.now_wall();
        let deadline = Deadline::from_now(now, DurationMs::from_seconds(30))
            .ok_or_else(|| DomainError::invalid_argument("deadline overflow"))?;
        let idempotency_key = format!(
            "orphan-stop-{}-{}-{}",
            node_id,
            orphan.media_session_id,
            self.id_generator.generate_message_id()
        );
        let command = MediaNodeCommand {
            request_id: self.id_generator.generate_message_id().to_string(),
            tenant_id,
            media_session_id: orphan.media_session_id,
            media_binding_id: MediaBindingId::default(),
            media_node_id: node_id,
            media_node_instance_epoch: orphan.media_node_instance_epoch,
            operation_id: self.id_generator.generate_operation_id(),
            owner_epoch,
            source_node_id: self.source_node_id,
            deadline: Some(deadline),
            idempotency_key,
            contract_version: node.contract_version,
            payload: CommandPayload::StopMediaSession {
                media_session_id: orphan.media_session_id,
            },
        };
        self.media_port
            .execute(command, self.clock.as_ref())
            .await?;
        Ok(())
    }

    /// Resolves a non-zero owner epoch for an orphan session.
    pub(crate) async fn resolve_orphan_owner_epoch(
        &self,
        tenant_id: TenantId,
        device_id: Option<DeviceId>,
    ) -> OwnerEpoch {
        let Some(device_id) = device_id else {
            return OwnerEpoch(1);
        };
        match self.owner_resolver.resolve(tenant_id, device_id).await {
            Ok(Some(owner)) if owner.owner_epoch.0 > 0 => owner.owner_epoch,
            Ok(Some(_)) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    device_id = %device_id,
                    "resolved owner has zero epoch for orphan session; using sentinel"
                );
                OwnerEpoch(1)
            }
            Ok(None) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    device_id = %device_id,
                    "no owner resolved for orphan session; using sentinel owner epoch"
                );
                OwnerEpoch(1)
            }
            Err(e) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    device_id = %device_id,
                    "failed to resolve owner for orphan session: {e}; using sentinel owner epoch"
                );
                OwnerEpoch(1)
            }
        }
    }
}

/// Classifies a media node that is missing from the active list.
///
/// Returns `(gone, needs_verification)`:
/// - `gone` means the node has been explicitly removed (Left with expired lease
///   or not found) and its sessions should be migrated/failed.
/// - `needs_verification` means the node is still registered but its lease has
///   expired or it is unhealthy; the binding should be marked
///   `NeedsVerification` without immediately failing the session.
pub(crate) fn classify_inactive_node(node: Option<&MediaNode>, now: UtcTimestamp) -> (bool, bool) {
    let Some(node) = node else {
        return (true, false);
    };

    let lease_expired = match node.lease_until {
        None => true,
        Some(lease) => now >= lease,
    };

    match node.status {
        NodeStatus::Left if lease_expired => (true, false),
        NodeStatus::Left => (false, false), // still in protection window
        _ if lease_expired || node.health == MediaNodeHealth::Unhealthy => (false, true),
        _ => (false, false),
    }
}

//! Media session reconciliation with media node reports.

use crate::dto::ReconciliationReport;
use crate::media_service::*;
use crate::media_service_helpers::*;
use crate::media_service_reconciliation_support::*;
use cheetah_domain::{
    DomainError, MediaBinding, MediaBindingError, MediaBindingState, MediaNode,
    MediaNodeSessionRef, MediaPurpose, MediaReservation, MediaSession, MediaSessionDesiredState,
    MediaSessionError, MediaSessionState, NodeStatus, UnitOfWork,
};
use cheetah_signal_types::{
    DurationMs, MediaBindingId, MediaSessionId, NodeId, PageRequest, RequestContext,
};
use std::collections::{BTreeMap, BTreeSet};

impl MediaService {
    /// Forces cleanup of all active sessions on the given media node by marking
    /// their bindings as failed and releasing scheduler reservations.
    pub async fn force_cleanup_node(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        node_id: NodeId,
    ) -> crate::Result<u64> {
        let tenant_id = context.tenant_id;
        let mut cleaned: u64 = 0;
        let mut cursor: Option<String> = None;

        loop {
            let request = match cursor {
                None => PageRequest::new(1000)?,
                Some(c) => PageRequest::new(1000)?.with_cursor(c),
            };
            let page = self
                .media_port
                .list_sessions(tenant_id, node_id, request, self.clock.as_ref())
                .await?;

            for r in page.items {
                let Some(mut session) = uow
                    .media_session_repository()
                    .get(tenant_id, r.media_session_id)
                    .await?
                else {
                    continue;
                };
                let Some(mut binding) = uow
                    .media_binding_repository()
                    .get_by_media_session(tenant_id, r.media_session_id)
                    .await?
                else {
                    continue;
                };

                if binding.is_terminal() {
                    continue;
                }
                if binding.media_node_id() != node_id
                    || binding.media_node_instance_epoch() != r.media_node_instance_epoch
                {
                    continue;
                }

                self.fail_session(
                    context,
                    uow,
                    &mut session,
                    &mut binding,
                    "forced_cleanup",
                    "admin forced cleanup",
                )
                .await?;
                // Commit the failed binding/session before the scheduler RPC so the
                // SQLite write lock is not held across the network call.
                uow.commit().await?;
                if let Err(e) = self
                    .media_port
                    .release(tenant_id, binding.media_binding_id(), self.clock.as_ref())
                    .await
                {
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        binding_id = %binding.media_binding_id(),
                        "failed to release scheduler reservation during forced cleanup: {e}"
                    );
                }
                cleaned += 1;
            }

            match page.next_cursor {
                None => break,
                Some(c) => cursor = Some(c),
            }
        }

        uow.commit().await?;
        self.media_port.record_forced_cleanup(cleaned);
        Ok(cleaned)
    }

    /// Reconciles local media session and binding state with the sessions
    /// currently reported by each media node.
    pub async fn reconcile(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
    ) -> crate::Result<ReconciliationReport> {
        let tenant_id = context.tenant_id;
        let mut report = ReconciliationReport::default();
        let mut to_release: Vec<(MediaSession, MediaBinding)> = Vec::new();
        let mut active_by_node: BTreeMap<NodeId, Vec<(MediaSession, MediaBinding)>> =
            BTreeMap::new();
        let mut reservations_to_release: Vec<MediaBindingId> = Vec::new();

        let mut cursor: Option<String> = None;
        loop {
            let request = match cursor {
                None => PageRequest::new(1000)?,
                Some(c) => PageRequest::new(1000)?.with_cursor(c),
            };
            let page = uow
                .media_session_repository()
                .list(tenant_id, None, None, None, None, request)
                .await?;
            for session in page.items {
                if let Some(binding) = uow
                    .media_binding_repository()
                    .get_by_media_session(tenant_id, session.media_session_id())
                    .await?
                {
                    if binding.is_terminal() {
                        continue;
                    }
                    if session.desired_state() == MediaSessionDesiredState::Stopped
                        || session.is_terminal()
                    {
                        to_release.push((session, binding));
                    } else {
                        active_by_node
                            .entry(binding.media_node_id())
                            .or_default()
                            .push((session, binding));
                    }
                }
            }
            match page.next_cursor {
                None => break,
                Some(c) => cursor = Some(c),
            }
        }

        for (session, binding) in &mut to_release {
            self.release_binding(context, uow, session, binding).await?;
            reservations_to_release.push(binding.media_binding_id());
            report.missing_released += 1;
        }
        // Persist terminal bindings/sessions before querying media nodes so the
        // SQLite write lock is not held across network RPCs.
        uow.commit().await?;

        let nodes: BTreeMap<NodeId, MediaNode> = self
            .media_port
            .list_nodes(tenant_id, self.clock.as_ref())
            .await?
            .into_iter()
            .map(|n| (n.node_id, n))
            .collect();
        let now = self.clock.now_wall();

        // Only query media nodes that actually host this tenant's active bindings.
        // This avoids O(media_nodes) RPC fan-out per tenant on every reconcile tick
        // while still checking every binding that matters.
        let local_node_ids: BTreeSet<NodeId> = active_by_node.keys().copied().collect();
        let node_ids: Vec<NodeId> = local_node_ids.iter().copied().collect();
        for node_id in node_ids {
            let Some(node) = nodes.get(&node_id) else {
                // Node is no longer reported as active; the second pass below
                // will classify it as gone or still in a protection window.
                continue;
            };
            report.nodes_scanned += 1;

            // A deregistered node still within its protection lease is kept in
            // the active list so the reconciler sees its bindings, but we must
            // not query or migrate those sessions until the window expires.
            if node.status == NodeStatus::Left
                && let Some(lease) = node.lease_until
                && now < lease
            {
                continue;
            }

            let local_list = active_by_node.remove(&node_id).unwrap_or_default();
            let local_ids: BTreeSet<MediaSessionId> = local_list
                .iter()
                .map(|(session, _)| session.media_session_id())
                .collect();

            let mut reported: BTreeMap<MediaSessionId, MediaNodeSessionRef> = BTreeMap::new();
            let mut cursor: Option<String> = None;
            loop {
                let request = match cursor {
                    None => PageRequest::new(1000)?,
                    Some(c) => PageRequest::new(1000)?.with_cursor(c),
                };
                let page = self
                    .media_port
                    .list_sessions(tenant_id, node_id, request, self.clock.as_ref())
                    .await?;
                for r in page.items {
                    reported.insert(r.media_session_id, r);
                }
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            report.sessions_found += reported.len() as u64;

            for (mut session, mut binding) in local_list {
                if node.draining {
                    self.migrate_or_fail(
                        context,
                        uow,
                        &mut session,
                        &mut binding,
                        "node_draining",
                        "media node is draining",
                        &mut report,
                    )
                    .await?;
                    continue;
                }
                match reported.get(&session.media_session_id()) {
                    Some(report_ref) => {
                        if report_ref.media_node_instance_epoch
                            != binding.media_node_instance_epoch()
                        {
                            self.fail_session(
                                context,
                                uow,
                                &mut session,
                                &mut binding,
                                "stale_node_instance",
                                "media node instance epoch mismatch",
                            )
                            .await?;
                            reservations_to_release.push(binding.media_binding_id());
                            report.missing_failed += 1;
                        } else {
                            self.converge_active(context, uow, &mut session, &mut binding)
                                .await?;
                        }
                    }
                    None => {
                        self.migrate_or_fail(
                            context,
                            uow,
                            &mut session,
                            &mut binding,
                            "reconciliation_missing",
                            "active media session missing on media node",
                            &mut report,
                        )
                        .await?;
                    }
                }
            }

            for id in reported.keys() {
                if !local_ids.contains(id) {
                    report.orphans_detected += 1;
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        node_id = %node_id,
                        media_session_id = %id,
                        "orphan media session reported by node"
                    );
                }
            }
            // Persist per-node state before the next media-node query.
            uow.commit().await?;
        }

        // Scan active media nodes that have no local binding for orphan sessions.
        // We only query nodes whose reported session count is non-zero, so idle
        // nodes do not cause fan-out.
        for (node_id, node) in &nodes {
            if local_node_ids.contains(node_id) || node.session_count == 0 {
                continue;
            }
            // Commit any pending writes before the next media-node RPC.
            uow.commit().await?;

            let mut orphan_cursor: Option<String> = None;
            loop {
                let request = match orphan_cursor {
                    None => PageRequest::new(1000)?,
                    Some(c) => PageRequest::new(1000)?.with_cursor(c),
                };
                let page = self
                    .media_port
                    .list_sessions(tenant_id, *node_id, request, self.clock.as_ref())
                    .await?;
                orphan_cursor = page.next_cursor;
                for orphan in page.items {
                    report.orphans_detected += 1;
                    match self
                        .stop_orphan_session(tenant_id, *node_id, node, &orphan)
                        .await
                    {
                        Ok(()) => {
                            report.orphans_stopped += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                tenant_id = %tenant_id,
                                node_id = %node_id,
                                media_session_id = %orphan.media_session_id,
                                "failed to stop orphan media session: {e}"
                            );
                        }
                    }
                }
                if orphan_cursor.is_none() {
                    break;
                }
            }
        }

        // Any sessions still in active_by_node are bound to media nodes that are
        // no longer active in the cluster (crashed, deregistered, or expired).
        for (node_id, sessions) in active_by_node {
            let node = self
                .media_port
                .get_node(node_id, self.clock.as_ref())
                .await?;
            let now = self.clock.now_wall();
            let (gone, needs_verification) = classify_inactive_node(node.as_ref(), now);

            if !gone && !needs_verification {
                // The node is still in a protection window (Left but lease valid).
                // Leave the binding/session alone until the window expires.
                tracing::info!(
                    tenant_id = %tenant_id,
                    node_id = %node_id,
                    "skipping reconciliation for protected media node"
                );
                continue;
            }

            for (mut session, mut binding) in sessions {
                let session_active = session.state() == MediaSessionState::Active;
                let binding_state = binding.state();
                let binding_active = binding_state == MediaBindingState::Active;
                let binding_needs_verification =
                    binding_state == MediaBindingState::NeedsVerification;

                if gone {
                    // Node has deregistered or is unknown. Migrate to a healthy node or fail.
                    self.migrate_or_fail(
                        context,
                        uow,
                        &mut session,
                        &mut binding,
                        "node_unavailable",
                        "media node no longer active",
                        &mut report,
                    )
                    .await?;
                } else if needs_verification {
                    if session_active && binding_active {
                        self.mark_binding_needs_verification(
                            context,
                            uow,
                            &mut session,
                            &mut binding,
                            "node_lease_expired",
                            "media node lease expired or is unhealthy; binding needs verification",
                            &mut report,
                        )
                        .await?;
                    } else if session_active && binding_needs_verification {
                        // Already verifying. Escalate to migrate/fail once the grace window
                        // expires so a crashed-but-not-deregistered node cannot leave
                        // sessions stuck indefinitely.
                        let grace_deadline = binding.updated_at().checked_add(
                            DurationMs::from_millis(self.needs_verification_grace_ms as i64),
                        );
                        if grace_deadline.is_some_and(|deadline| now >= deadline) {
                            self.migrate_or_fail(
                                context,
                                uow,
                                &mut session,
                                &mut binding,
                                "node_verification_grace_expired",
                                "media node remained unhealthy beyond needs-verification grace period",
                                &mut report,
                            )
                            .await?;
                        }
                    } else {
                        // Setup-phase sessions cannot wait in NeedsVerification;
                        // attempt to migrate to a healthy node or fail.
                        self.migrate_or_fail(
                            context,
                            uow,
                            &mut session,
                            &mut binding,
                            "node_unavailable_setup",
                            "media node unhealthy during setup",
                            &mut report,
                        )
                        .await?;
                    }
                } else {
                    // Node is in a protection window; nothing to do.
                }
            }
            // Persist per-node state before the next inactive-node lookup.
            uow.commit().await?;
        }
        // Persist migrations/failures before releasing scheduler reservations.
        uow.commit().await?;

        for binding_id in reservations_to_release {
            if let Err(e) = self
                .media_port
                .release(tenant_id, binding_id, self.clock.as_ref())
                .await
            {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    binding_id = %binding_id,
                    "failed to release scheduler reservation after reconciliation: {e}"
                );
            }
        }

        uow.commit().await?;

        self.media_port.record_reconcile(
            report.nodes_scanned,
            report
                .missing_released
                .saturating_add(report.migrations_succeeded)
                .saturating_add(report.needs_verification),
            report
                .missing_failed
                .saturating_add(report.migrations_failed),
            report.orphans_stopped,
        );

        Ok(report)
    }

    async fn release_binding(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        session: &mut MediaSession,
        binding: &mut MediaBinding,
    ) -> crate::Result<()> {
        if !session.is_terminal() {
            let ev = session.stop(self.clock.as_ref())?;
            append_session_event(self, context, uow, session, ev).await?;
            uow.media_session_repository().save(session).await?;
        }

        if !binding.is_terminal() {
            if binding.state() != MediaBindingState::Releasing {
                let ev = binding.release(self.clock.as_ref())?;
                append_binding_event(self, context, uow, binding, ev).await?;
                uow.media_binding_repository().save(binding).await?;
            }
            let ev = binding.released(self.clock.as_ref())?;
            append_binding_event(self, context, uow, binding, ev).await?;
            uow.media_binding_repository().save(binding).await?;
        }

        Ok(())
    }

    async fn fail_session(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        session: &mut MediaSession,
        binding: &mut MediaBinding,
        code: &str,
        message: &str,
    ) -> crate::Result<()> {
        if !session.is_terminal() {
            let ev = session.failed(MediaSessionError::new(code, message), self.clock.as_ref())?;
            append_session_event(self, context, uow, session, ev).await?;
            uow.media_session_repository().save(session).await?;
        }

        if !binding.is_terminal() {
            let ev = binding.failed(MediaBindingError::new(code, message), self.clock.as_ref())?;
            append_binding_event(self, context, uow, binding, ev).await?;
            uow.media_binding_repository().save(binding).await?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn migrate_or_fail(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        session: &mut MediaSession,
        binding: &mut MediaBinding,
        code: &str,
        message: &str,
        report: &mut ReconciliationReport,
    ) -> crate::Result<()> {
        let tenant_id = context.tenant_id;

        // Flush any pending writes from earlier sessions in this node batch before
        // the next media-node RPC, then re-acquire a read transaction only for
        // device/channel lookup.
        uow.commit().await?;

        let (device, channel) = match self
            .ensure_device_and_channel_ready(
                uow,
                tenant_id,
                session.device_id(),
                session.channel_id(),
            )
            .await
        {
            Ok(tuple) => tuple,
            Err(_) => {
                if !session.is_terminal() {
                    self.fail_session(context, uow, session, binding, code, message)
                        .await?;
                    uow.commit().await?;
                }
                if let Err(e2) = self
                    .media_port
                    .release(tenant_id, binding.media_binding_id(), self.clock.as_ref())
                    .await
                {
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        binding_id = %binding.media_binding_id(),
                        "failed to release scheduler reservation after device/channel check failure: {e2}"
                    );
                }
                report.migrations_failed += 1;
                return Ok(());
            }
        };

        let requirements = build_media_requirements(
            &device,
            &channel,
            session.purpose(),
            session.media_session_id(),
            std::collections::BTreeMap::new(),
        );
        let media_binding_id = self.id_generator.generate_media_binding_id();

        // Commit before the reservation network call so the DB write lock is not
        // held across the media-node RPC.
        uow.commit().await?;

        let reservation: crate::Result<MediaReservation> = match session.purpose() {
            MediaPurpose::Live => {
                self.media_port
                    .reserve_live(
                        tenant_id,
                        session.device_id(),
                        session.channel_id(),
                        session.media_session_id(),
                        media_binding_id,
                        MediaPurpose::Live,
                        &requirements,
                        self.clock.as_ref(),
                    )
                    .await
            }
            MediaPurpose::Playback => {
                let now = self.clock.now_wall();
                let start_time = session.playback_start_time().unwrap_or(now);
                let end_time = session.playback_end_time().unwrap_or(now);
                let scale = session.playback_scale().unwrap_or(1.0);
                self.media_port
                    .reserve_playback(
                        tenant_id,
                        session.device_id(),
                        session.channel_id(),
                        session.media_session_id(),
                        media_binding_id,
                        start_time,
                        end_time,
                        scale,
                        &requirements,
                        self.clock.as_ref(),
                    )
                    .await
            }
            MediaPurpose::Talk => {
                self.media_port
                    .reserve_talk(
                        tenant_id,
                        session.device_id(),
                        session.channel_id(),
                        session.media_session_id(),
                        media_binding_id,
                        &requirements,
                        self.clock.as_ref(),
                    )
                    .await
            }
            MediaPurpose::Broadcast => {
                self.media_port
                    .reserve_broadcast(
                        tenant_id,
                        session.device_id(),
                        session.channel_id(),
                        session.media_session_id(),
                        media_binding_id,
                        &requirements,
                        self.clock.as_ref(),
                    )
                    .await
            }
            _ => Err(DomainError::invalid_argument(
                "unknown media purpose for migration",
            )),
        }
        .map_err(crate::SignalError::from);

        let reservation = match reservation {
            Ok(reservation) => reservation,
            Err(_) => {
                if !session.is_terminal() {
                    self.fail_session(context, uow, session, binding, code, message)
                        .await?;
                    uow.commit().await?;
                }
                if let Err(e2) = self
                    .media_port
                    .release(tenant_id, binding.media_binding_id(), self.clock.as_ref())
                    .await
                {
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        binding_id = %binding.media_binding_id(),
                        "failed to release scheduler reservation after migration failure: {e2}"
                    );
                }
                report.migrations_failed += 1;
                return Ok(());
            }
        };

        // Only bump generation after a successful reservation so the new
        // binding is in a fresh generation and old callbacks cannot advance it.
        let ev = session.bump_generation(self.clock.as_ref())?;
        append_session_event(self, context, uow, session, ev).await?;
        uow.media_session_repository().save(session).await?;
        // Persist the bumped generation before issuing media-node RPCs.
        uow.commit().await?;

        let command_result = self
            .dispatch_reconnect_command(
                context,
                uow,
                session,
                binding,
                session.device_id(),
                session.channel_id(),
                media_binding_id,
                &reservation,
            )
            .await;
        // Persist any post-command state before the scheduler release RPC.
        uow.commit().await?;

        if let Err(e) = self
            .media_port
            .release(tenant_id, binding.media_binding_id(), self.clock.as_ref())
            .await
        {
            tracing::warn!(
                tenant_id = %tenant_id,
                binding_id = %binding.media_binding_id(),
                "failed to release scheduler reservation after migration attempt: {e}"
            );
        }

        match command_result {
            Ok(_) => {
                report.migrations_succeeded += 1;
            }
            Err(e) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    media_session_id = %session.media_session_id(),
                    "migration failed: {e}"
                );
                report.migrations_failed += 1;
            }
        }

        Ok(())
    }
}

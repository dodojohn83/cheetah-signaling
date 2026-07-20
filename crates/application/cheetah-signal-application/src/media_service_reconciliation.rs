//! Media session reconciliation with media node reports.

use crate::dto::ReconciliationReport;
use crate::media_service::*;
use cheetah_domain::{
    DomainError, MediaBinding, MediaBindingError, MediaBindingState, MediaNodeSessionRef,
    MediaPurpose, MediaReservation, MediaSession, MediaSessionDesiredState, MediaSessionError,
    MediaSessionState, UnitOfWork,
};
use cheetah_signal_types::{MediaBindingId, MediaSessionId, NodeId, PageRequest, RequestContext};
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
        self.media_port.record_reconcile(1, 0, 0, cleaned);
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

        let nodes = self
            .media_port
            .list_nodes(tenant_id, self.clock.as_ref())
            .await?;
        report.nodes_scanned = nodes.len() as u64;

        for node in nodes {
            let node_id = node.node_id;
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
        }

        // Any sessions still in active_by_node are bound to media nodes that are
        // no longer active in the cluster (crashed, deregistered, or expired).
        for (_node_id, sessions) in active_by_node {
            for (mut session, mut binding) in sessions {
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
            }
        }

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
                .saturating_add(report.migrations_succeeded),
            report
                .missing_failed
                .saturating_add(report.migrations_failed),
            report.orphans_detected,
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

        let reservation: crate::Result<(MediaReservation, MediaBindingId)> = async {
            let (device, channel) = self
                .ensure_device_and_channel_ready(
                    uow,
                    tenant_id,
                    session.device_id(),
                    session.channel_id(),
                )
                .await?;
            let requirements = build_media_requirements(
                &device,
                &channel,
                session.purpose(),
                session.media_session_id(),
                std::collections::BTreeMap::new(),
            );
            let media_binding_id = self.id_generator.generate_media_binding_id();
            let reservation = match session.purpose() {
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
                _ => Err(DomainError::invalid_argument(
                    "unknown media purpose for migration",
                )),
            }?;
            Ok((reservation, media_binding_id))
        }
        .await;

        let (reservation, media_binding_id) = match reservation {
            Ok(tuple) => tuple,
            Err(_) => {
                if !session.is_terminal() {
                    self.fail_session(context, uow, session, binding, code, message)
                        .await?;
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

    pub(crate) async fn converge_active(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
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
        }

        Ok(())
    }
}

async fn append_session_event(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &MediaSession,
    event: cheetah_domain::DomainEvent,
) -> crate::Result<()> {
    uow.outbox()
        .append(wrap_event(
            service.id_generator.as_ref(),
            service.clock.as_ref(),
            context,
            context.tenant_id,
            media_session_resource_ref(context.tenant_id, session.media_session_id()),
            session.revision().0,
            event,
        ))
        .await?;
    Ok(())
}

async fn append_binding_event(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    binding: &MediaBinding,
    event: cheetah_domain::DomainEvent,
) -> crate::Result<()> {
    uow.outbox()
        .append(wrap_event(
            service.id_generator.as_ref(),
            service.clock.as_ref(),
            context,
            context.tenant_id,
            media_binding_resource_ref(context.tenant_id, binding.media_binding_id()),
            binding.revision().0,
            event,
        ))
        .await?;
    Ok(())
}

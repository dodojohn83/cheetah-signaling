//! Reconciles `MediaBinding` state with actual media-node resources.

use crate::media_reconciliation::{converge_active, fail_session_and_binding};
use cheetah_domain::{
    MediaBinding, MediaNodeSessionRef, MediaPort, MediaSession, MediaSessionDesiredState,
    UnitOfWork,
};
use cheetah_signal_types::{
    Clock, IdGenerator, MAX_PAGE_SIZE, MediaBindingId, MediaSessionId, NodeId, PageRequest,
    RequestContext,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Report produced by a `MediaBindingReconciler` pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaBindingReconciliationReport {
    /// Number of media nodes scanned.
    pub nodes_scanned: u64,
    /// Number of active sessions reported by media nodes.
    pub sessions_found: u64,
    /// Number of bindings that were marked failed.
    pub missing_failed: u64,
    /// Number of reported sessions without a local binding.
    pub orphans_detected: u64,
    /// Scheduler reservations that the caller should release after committing.
    pub reservations_to_release: Vec<MediaBindingId>,
}

/// Reconciles database media bindings and sessions against the sessions
/// currently reported by each media node.
#[derive(Clone)]
pub struct MediaBindingReconciler {
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    media_port: Arc<dyn MediaPort>,
    batch_size: u32,
}

impl std::fmt::Debug for MediaBindingReconciler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaBindingReconciler")
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

impl MediaBindingReconciler {
    /// Creates a new reconciler.
    pub fn new(
        clock: Arc<dyn Clock>,
        id_generator: Arc<dyn IdGenerator>,
        media_port: Arc<dyn MediaPort>,
        batch_size: u32,
    ) -> Self {
        Self {
            clock,
            id_generator,
            media_port,
            batch_size: batch_size.clamp(1, MAX_PAGE_SIZE),
        }
    }

    /// Scans active media sessions and compares them with the media-node
    /// reports. Sessions that are no longer present, belong to a deregistered
    /// node, or have an instance-epoch mismatch are failed.
    ///
    /// The caller is responsible for committing `uow` and releasing any
    /// `reservations_to_release` returned in the report.
    pub async fn reconcile(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
    ) -> crate::Result<MediaBindingReconciliationReport> {
        let tenant_id = context.tenant_id;
        let mut report = MediaBindingReconciliationReport::default();
        let mut active_by_node: BTreeMap<NodeId, Vec<(MediaSession, MediaBinding)>> =
            BTreeMap::new();

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

            for session in page.items {
                if session.is_terminal()
                    || session.desired_state() == MediaSessionDesiredState::Stopped
                {
                    continue;
                }
                if let Some(binding) = uow
                    .media_binding_repository()
                    .get_by_media_session(tenant_id, session.media_session_id())
                    .await?
                {
                    if binding.is_terminal() {
                        continue;
                    }
                    active_by_node
                        .entry(binding.media_node_id())
                        .or_default()
                        .push((session, binding));
                }
            }

            match page.next_cursor {
                None => break,
                Some(c) => cursor = Some(c),
            }
        }

        let nodes = self
            .media_port
            .list_nodes(tenant_id, self.clock.as_ref())
            .await?;
        report.nodes_scanned = nodes.len() as u64;

        for node_id in nodes {
            let local_list = active_by_node.remove(&node_id).unwrap_or_default();
            let local_ids: BTreeSet<MediaSessionId> = local_list
                .iter()
                .map(|(session, _)| session.media_session_id())
                .collect();

            let mut reported: BTreeMap<MediaSessionId, MediaNodeSessionRef> = BTreeMap::new();
            let mut node_cursor: Option<String> = None;
            loop {
                let request = match node_cursor {
                    None => PageRequest::new(self.batch_size)?,
                    Some(c) => PageRequest::new(self.batch_size)?.with_cursor(c),
                };
                let page = self
                    .media_port
                    .list_sessions(tenant_id, node_id, request, self.clock.as_ref())
                    .await?;
                for r in page.items {
                    reported.insert(r.media_session_id, r);
                }
                node_cursor = page.next_cursor;
                if node_cursor.is_none() {
                    break;
                }
            }
            report.sessions_found += reported.len() as u64;

            for (mut session, mut binding) in local_list {
                let session_rev_before = session.revision().0;
                let binding_rev_before = binding.revision().0;
                match reported.get(&session.media_session_id()) {
                    Some(report_ref) => {
                        if report_ref.media_node_instance_epoch
                            != binding.media_node_instance_epoch()
                        {
                            fail_session_and_binding(
                                self.id_generator.as_ref(),
                                self.clock.as_ref(),
                                context,
                                uow,
                                &mut session,
                                &mut binding,
                                "stale_node_instance",
                                "media node instance epoch mismatch",
                            )
                            .await?;
                            report.missing_failed += 1;
                            report
                                .reservations_to_release
                                .push(binding.media_binding_id());
                        } else {
                            converge_active(
                                self.id_generator.as_ref(),
                                self.clock.as_ref(),
                                context,
                                uow,
                                &mut session,
                                &mut binding,
                            )
                            .await?;
                        }
                    }
                    None => {
                        fail_session_and_binding(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            uow,
                            &mut session,
                            &mut binding,
                            "reconciliation_missing",
                            "active media session missing on media node",
                        )
                        .await?;
                        report.missing_failed += 1;
                        report
                            .reservations_to_release
                            .push(binding.media_binding_id());
                    }
                }
                if session.revision().0 != session_rev_before {
                    uow.media_session_repository().save(&session).await?;
                }
                if binding.revision().0 != binding_rev_before {
                    uow.media_binding_repository().save(&binding).await?;
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
                let session_rev_before = session.revision().0;
                let binding_rev_before = binding.revision().0;
                fail_session_and_binding(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    uow,
                    &mut session,
                    &mut binding,
                    "node_unavailable",
                    "media node no longer active",
                )
                .await?;
                if session.revision().0 != session_rev_before {
                    uow.media_session_repository().save(&session).await?;
                }
                if binding.revision().0 != binding_rev_before {
                    uow.media_binding_repository().save(&binding).await?;
                }
                report
                    .reservations_to_release
                    .push(binding.media_binding_id());
                report.missing_failed += 1;
            }
        }

        Ok(report)
    }
}

//! Owner reconciler.
//!
//! Compares the database owner lease for a set of local device sessions and
//! reports sessions that are no longer valid for this node. The caller
//! (typically the runtime) is responsible for closing orphaned sessions.

use cheetah_domain::OwnerInfo;
use cheetah_signal_types::{
    Clock, DeviceId, MAX_PAGE_SIZE, NodeId, PageRequest, TenantId, UtcTimestamp,
};
use cheetah_storage_api::OwnerRepository;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::warn;

/// A device session held locally by this node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalDeviceSession {
    /// Tenant of the device.
    pub tenant_id: TenantId,
    /// Device identifier.
    pub device_id: DeviceId,
    /// Owner information cached by the local session, if known.
    pub local_owner: Option<OwnerInfo>,
}

/// Result of an owner reconciliation pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OwnerReconciliationReport {
    /// Number of local sessions scanned.
    pub scanned: u64,
    /// Number of local sessions whose database owner still matches this node.
    pub matched: u64,
    /// Number of local sessions whose database owner is missing, expired, or
    /// belongs to another node/epoch.
    pub orphaned: u64,
    /// Orphaned sessions that should be released by the runtime.
    pub orphaned_sessions: Vec<LocalDeviceSession>,
}

/// Result of a database owner scan for missing local sessions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OwnerMissingReport {
    /// Database owner entries scanned in this page.
    pub db_scanned: u64,
    /// Devices owned by this node in the database that have no local session.
    pub missing: u64,
    /// Missing sessions that should be started by the runtime.
    pub missing_sessions: Vec<LocalDeviceSession>,
    /// Cursor for the next page, if more database owners exist.
    pub next_cursor: Option<String>,
}

/// Reconciles database owner leases against local device sessions.
#[derive(Clone)]
pub struct OwnerReconciler {
    this_node: NodeId,
    owner_repository: Arc<dyn OwnerRepository>,
    clock: Arc<dyn Clock>,
    batch_size: usize,
}

impl std::fmt::Debug for OwnerReconciler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerReconciler")
            .field("this_node", &self.this_node)
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

impl OwnerReconciler {
    /// Creates a new owner reconciler.
    pub fn new(
        this_node: NodeId,
        owner_repository: Arc<dyn OwnerRepository>,
        clock: Arc<dyn Clock>,
        batch_size: usize,
    ) -> Self {
        Self {
            this_node,
            owner_repository,
            clock,
            batch_size: batch_size.clamp(1, MAX_PAGE_SIZE as usize),
        }
    }

    /// Validates up to `batch_size` local sessions against the database owner.
    ///
    /// The caller should pass a batch of local sessions and repeatedly invoke
    /// the reconciler until all sessions have been processed.
    pub async fn reconcile(
        &self,
        local_sessions: &[LocalDeviceSession],
    ) -> crate::Result<OwnerReconciliationReport> {
        let now = self.clock.now_wall();
        let mut report = OwnerReconciliationReport::default();

        for session in local_sessions.iter().take(self.batch_size) {
            report.scanned += 1;

            let owner = self
                .owner_repository
                .get(session.tenant_id, session.device_id)
                .await
                .map_err(|e| crate::SignalError::from(cheetah_domain::DomainError::from(e)))?;

            if let Some(owner) = owner {
                if is_valid_local_owner(&owner, session, self.this_node, now) {
                    report.matched += 1;
                } else {
                    warn!(
                        tenant_id = %session.tenant_id.as_uuid(),
                        device_id = %session.device_id.as_uuid(),
                        owner_node_id = %owner.owner_node_id.as_uuid(),
                        owner_epoch = owner.owner_epoch.0,
                        "local device session no longer matches database owner"
                    );
                    report.orphaned += 1;
                    report.orphaned_sessions.push(session.clone());
                }
            } else {
                warn!(
                    tenant_id = %session.tenant_id.as_uuid(),
                    device_id = %session.device_id.as_uuid(),
                    "local device session has no database owner"
                );
                report.orphaned += 1;
                report.orphaned_sessions.push(session.clone());
            }
        }

        Ok(report)
    }

    /// Scans one page of database owners assigned to this node and reports
    /// devices that are missing a local session.
    ///
    /// The caller should repeatedly invoke this method with the `next_cursor`
    /// from the previous report until `next_cursor` is `None`.
    pub async fn find_missing(
        &self,
        local_sessions: &[LocalDeviceSession],
        cursor: Option<String>,
    ) -> crate::Result<OwnerMissingReport> {
        let mut page_request = PageRequest::new(self.batch_size as u32)?;
        page_request.cursor = cursor;

        let page = self
            .owner_repository
            .list_by_node(self.this_node, page_request)
            .await
            .map_err(|e| crate::SignalError::from(cheetah_domain::DomainError::from(e)))?;

        let local_set: HashSet<(TenantId, DeviceId)> = local_sessions
            .iter()
            .map(|s| (s.tenant_id, s.device_id))
            .collect();

        let mut report = OwnerMissingReport {
            next_cursor: page.next_cursor,
            ..OwnerMissingReport::default()
        };

        for owned in page.items {
            report.db_scanned += 1;
            if !local_set.contains(&(owned.tenant_id, owned.device_id)) {
                report.missing += 1;
                report.missing_sessions.push(LocalDeviceSession {
                    tenant_id: owned.tenant_id,
                    device_id: owned.device_id,
                    local_owner: Some(owned.owner),
                });
            }
        }

        Ok(report)
    }
}

fn is_valid_local_owner(
    owner: &OwnerInfo,
    session: &LocalDeviceSession,
    this_node: NodeId,
    now: UtcTimestamp,
) -> bool {
    if owner.owner_node_id != this_node {
        return false;
    }

    if owner.lease_until.is_some_and(|lease| lease <= now) {
        return false;
    }

    if let Some(local) = &session.local_owner {
        if local.owner_node_id != owner.owner_node_id {
            return false;
        }
        if local.owner_epoch.0 != owner.owner_epoch.0 {
            return false;
        }
    }

    true
}

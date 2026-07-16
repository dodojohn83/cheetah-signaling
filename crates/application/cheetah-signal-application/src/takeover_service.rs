//! Fault takeover service for device ownership.
//!
//! When a device reconnects to a node after its previous owner has failed,
//! this service competes for ownership, recovers any non-terminal operations
//! that were bound to the old epoch, and emits an `OwnerChanged` domain event
//! so protocol drivers can rebuild sessions and report gaps.

use std::sync::Arc;

use cheetah_domain::{DomainEvent, OwnerInfo};
use cheetah_signal_types::{
    Clock, DeviceId, DurationMs, Event, IdGenerator, NodeId, RequestContext, ResourceId,
    ResourceKind, ResourceRef, TenantId, UtcTimestamp,
};
use cheetah_storage_api::{Storage, StorageError};

fn storage_to_signal(err: StorageError) -> crate::SignalError {
    cheetah_domain::DomainError::from(err).into()
}

const OWNER_LOST_CODE: &str = "owner_lost";
const OWNER_LOST_MESSAGE: &str = "device owner epoch advanced due to takeover";
const RECOVERY_STATUS_PENDING: &str = "pending";
const RECOVERY_STATUS_RUNNING: &str = "running";

/// Result of a fault takeover attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TakeoverResult {
    /// This node is already the valid owner; no state changed.
    Current,
    /// Ownership was taken over by this node; carries recovered operation count.
    Acquired {
        /// New owner information.
        owner: OwnerInfo,
        /// Number of stale operations that were failed due to the epoch advance.
        recovered: usize,
    },
    /// Ownership belongs to a different (live) node.
    RemoteOwner {
        /// Current owner information.
        owner: OwnerInfo,
    },
    /// This node is not eligible to take ownership.
    NotEligible,
}

/// Validates a locally cached owner record against the database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerValidation {
    /// Local owner still matches the database.
    Current,
    /// Local owner is fenced; the database has a higher epoch or different node.
    Fenced,
}

/// Application service that executes fault takeovers and fences stale owners.
#[derive(Clone)]
pub struct TakeoverService {
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    this_node: NodeId,
    lease_duration: DurationMs,
}

impl TakeoverService {
    /// Creates a new takeover service.
    pub fn new(
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
        id_generator: Arc<dyn IdGenerator>,
        this_node: NodeId,
        lease_duration: DurationMs,
    ) -> Self {
        Self {
            storage,
            clock,
            id_generator,
            this_node,
            lease_duration,
        }
    }

    /// Attempts to take ownership of `device_id` for this node.
    ///
    /// If the current owner is alive and eligible, the existing owner is
    /// returned without changing state. Otherwise this node tries to acquire
    /// ownership and recover stale operations.
    pub async fn takeover(
        &self,
        context: &RequestContext,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> crate::Result<TakeoverResult> {
        let now = self.clock.now_wall();
        let lease_until = now
            .checked_add(self.lease_duration)
            .ok_or_else(|| cheetah_domain::DomainError::internal("owner lease overflow"))?;

        let mut owner_repo = self.storage.owner_repository();
        let current = owner_repo
            .get(tenant_id, device_id)
            .await
            .map_err(storage_to_signal)?;

        if let Some(owner) = current.clone()
            && let Some(node) = self.get_node(owner.owner_node_id).await?
        {
            if is_node_alive(&node, now) && owner.owner_node_id == self.this_node {
                return Ok(TakeoverResult::Current);
            }
            if is_node_alive(&node, now) && owner.owner_node_id != self.this_node {
                return Ok(TakeoverResult::RemoteOwner { owner });
            }
        }

        if !self.is_this_node_eligible(now).await? {
            return Ok(TakeoverResult::NotEligible);
        }

        let owner = owner_repo
            .acquire(tenant_id, device_id, self.this_node, now, lease_until)
            .await
            .map_err(storage_to_signal)?;

        if owner.owner_node_id != self.this_node {
            return Ok(TakeoverResult::RemoteOwner { owner });
        }

        let mut uow = self.storage.begin().await.map_err(storage_to_signal)?;
        let recovered = self
            .recover_operations(context, uow.as_mut(), tenant_id, device_id, &owner)
            .await?;

        let event = wrap_event(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            tenant_id,
            device_resource_ref(tenant_id, device_id),
            owner.owner_epoch.0,
            DomainEvent::OwnerChanged {
                tenant_id,
                device_id,
                node_id: owner.owner_node_id,
                owner_epoch: owner.owner_epoch,
                previous_node_id: current.as_ref().map(|o| o.owner_node_id),
                previous_epoch: current.as_ref().map(|o| o.owner_epoch),
                takeover: current.is_none_or(|previous| {
                    previous.owner_node_id != owner.owner_node_id
                        || previous.owner_epoch < owner.owner_epoch
                }),
            },
        );
        uow.outbox().append(event).await?;
        uow.commit().await?;

        Ok(TakeoverResult::Acquired { owner, recovered })
    }

    /// Validates a locally cached owner against the database.
    pub async fn validate_local_owner(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        local_owner: &OwnerInfo,
    ) -> crate::Result<OwnerValidation> {
        let owner_repo = self.storage.owner_repository();
        let current = owner_repo
            .get(tenant_id, device_id)
            .await
            .map_err(storage_to_signal)?;

        match current {
            Some(owner)
                if owner.owner_node_id == local_owner.owner_node_id
                    && owner.owner_epoch == local_owner.owner_epoch =>
            {
                Ok(OwnerValidation::Current)
            }
            _ => Ok(OwnerValidation::Fenced),
        }
    }

    async fn get_node(
        &self,
        node_id: NodeId,
    ) -> crate::Result<Option<cheetah_domain::ClusterNode>> {
        let repo = self.storage.node_repository();
        repo.get(node_id).await.map_err(storage_to_signal)
    }

    async fn is_this_node_eligible(&self, now: UtcTimestamp) -> crate::Result<bool> {
        match self.get_node(self.this_node).await? {
            Some(node) => Ok(is_node_alive(&node, now) && !node.draining),
            None => Ok(false),
        }
    }

    async fn recover_operations(
        &self,
        context: &RequestContext,
        uow: &mut dyn cheetah_domain::UnitOfWork,
        tenant_id: TenantId,
        device_id: DeviceId,
        owner: &OwnerInfo,
    ) -> crate::Result<usize> {
        let mut recovered = 0;

        for status in [RECOVERY_STATUS_PENDING, RECOVERY_STATUS_RUNNING] {
            let mut cursor: Option<String> = None;
            loop {
                let mut page_request =
                    cheetah_signal_types::PageRequest::new(cheetah_signal_types::MAX_PAGE_SIZE)?;
                page_request.cursor = cursor;
                let page = uow
                    .operation_repository()
                    .list(
                        tenant_id,
                        Some(device_id),
                        Some(status.to_string()),
                        None,
                        page_request,
                    )
                    .await?;

                for mut operation in page.items {
                    if operation.expected_owner_epoch() < owner.owner_epoch {
                        let event = operation
                            .fail(OWNER_LOST_CODE, OWNER_LOST_MESSAGE, self.clock.as_ref())
                            .map_err(crate::SignalError::from)?;
                        uow.operation_repository().save(&operation).await?;
                        uow.outbox()
                            .append(wrap_event(
                                self.id_generator.as_ref(),
                                self.clock.as_ref(),
                                context,
                                tenant_id,
                                operation_resource_ref(tenant_id, operation.operation_id()),
                                operation.revision().0,
                                event,
                            ))
                            .await?;
                        recovered += 1;
                    }
                }

                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }

        Ok(recovered)
    }
}

impl std::fmt::Debug for TakeoverService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TakeoverService")
            .field("this_node", &self.this_node)
            .finish_non_exhaustive()
    }
}

fn is_node_alive(node: &cheetah_domain::ClusterNode, now: UtcTimestamp) -> bool {
    node.lease_until > now
}

fn device_resource_ref(tenant_id: TenantId, device_id: DeviceId) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    }
}

fn operation_resource_ref(
    tenant_id: TenantId,
    operation_id: cheetah_signal_types::OperationId,
) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Operation,
        id: ResourceId::Operation(operation_id),
    }
}

fn wrap_event(
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
    context: &RequestContext,
    tenant_id: TenantId,
    aggregate_ref: ResourceRef,
    aggregate_sequence: u64,
    payload: DomainEvent,
) -> Event<DomainEvent> {
    Event::new(
        id_generator,
        clock,
        context,
        tenant_id,
        aggregate_ref,
        aggregate_sequence,
        payload,
    )
}

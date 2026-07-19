//! Operation application service.

use crate::dto::{OperationDto, SubmitOperationRequest};
use cheetah_domain::{DomainError, DomainEvent, Operation, OperationResult, UnitOfWork};
use cheetah_signal_types::{
    Clock, Event, IdGenerator, OperationId, RequestContext, ResourceId, ResourceKind, ResourceRef,
    TenantId,
};

/// Application service for operation lifecycle.
#[derive(Clone)]
pub struct OperationService {
    clock: std::sync::Arc<dyn Clock>,
    id_generator: std::sync::Arc<dyn IdGenerator>,
}

impl OperationService {
    /// Creates a new operation service.
    pub fn new(
        clock: std::sync::Arc<dyn Clock>,
        id_generator: std::sync::Arc<dyn IdGenerator>,
    ) -> Self {
        Self {
            clock,
            id_generator,
        }
    }

    /// Submits an operation idempotently.
    pub async fn submit_operation(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        request: SubmitOperationRequest,
    ) -> crate::Result<OperationDto> {
        let tenant_id = context.tenant_id;
        let idempotency_key = request.idempotency_key;
        let target = request.target;
        if target.tenant_id != tenant_id {
            return Err(crate::SignalError::from(DomainError::invalid_argument(
                "target tenant does not match request context",
            )));
        }
        let scope = cheetah_domain::IdempotencyScope::new(
            tenant_id,
            context.principal.id.clone(),
            target.clone(),
            idempotency_key.clone(),
        )
        .map_err(crate::SignalError::from)?;

        if let Some(existing) = uow
            .operation_repository()
            .get_by_idempotency(&scope)
            .await?
        {
            return Ok(OperationDto::from(&existing));
        }

        let (operation, event) = Operation::new(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            idempotency_key,
            request.device_id,
            target,
            request.payload,
            request.deadline,
            request.expected_owner_epoch,
        )
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
        uow.commit().await?;

        Ok(OperationDto::from(&operation))
    }

    /// Gets an operation by id.
    pub async fn get_operation(
        &self,
        uow: &mut dyn UnitOfWork,
        tenant_id: TenantId,
        operation_id: OperationId,
    ) -> crate::Result<OperationDto> {
        let operation = uow
            .operation_repository()
            .get(tenant_id, operation_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "operation",
                    operation_id.to_string(),
                ))
            })?;
        Ok(OperationDto::from(&operation))
    }

    /// Cancels an operation.
    pub async fn cancel_operation(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        operation_id: OperationId,
    ) -> crate::Result<OperationDto> {
        let tenant_id = context.tenant_id;
        let mut operation = uow
            .operation_repository()
            .get(tenant_id, operation_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "operation",
                    operation_id.to_string(),
                ))
            })?;
        let event = operation
            .cancel(self.clock.as_ref())
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
        uow.commit().await?;
        Ok(OperationDto::from(&operation))
    }

    /// Completes an operation with a result.
    pub async fn complete_operation(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        operation_id: OperationId,
        result: OperationResult,
    ) -> crate::Result<OperationDto> {
        let tenant_id = context.tenant_id;
        let mut operation = uow
            .operation_repository()
            .get(tenant_id, operation_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "operation",
                    operation_id.to_string(),
                ))
            })?;
        let event = operation
            .complete(result, self.clock.as_ref())
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
        uow.commit().await?;
        Ok(OperationDto::from(&operation))
    }

    /// Times out an operation.
    pub async fn timeout_operation(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        operation_id: OperationId,
        error: cheetah_domain::OperationError,
    ) -> crate::Result<OperationDto> {
        let tenant_id = context.tenant_id;
        let mut operation = uow
            .operation_repository()
            .get(tenant_id, operation_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "operation",
                    operation_id.to_string(),
                ))
            })?;
        let event = operation
            .timeout(error, self.clock.as_ref())
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
        uow.commit().await?;
        Ok(OperationDto::from(&operation))
    }
}

impl std::fmt::Debug for OperationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationService").finish_non_exhaustive()
    }
}

pub(crate) fn wrap_event(
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

pub(crate) fn operation_resource_ref(tenant_id: TenantId, operation_id: OperationId) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Operation,
        id: ResourceId::Operation(operation_id),
    }
}

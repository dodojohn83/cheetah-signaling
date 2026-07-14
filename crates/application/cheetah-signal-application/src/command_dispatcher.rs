//! Command dispatcher.

use crate::dto::OperationDto;
use cheetah_domain::{
    CommandBus, DeviceOwnerResolver, DomainError, DomainEvent, OperationResult, UnitOfWork,
};
use cheetah_signal_types::{
    Clock, Event, IdGenerator, OperationId, RequestContext, ResourceId, ResourceKind, ResourceRef,
    TenantId,
};

/// Dispatches commands from pending operations to the command bus.
#[derive(Clone)]
pub struct CommandDispatcher {
    clock: std::sync::Arc<dyn Clock>,
    id_generator: std::sync::Arc<dyn IdGenerator>,
    owner_resolver: std::sync::Arc<dyn DeviceOwnerResolver>,
    command_bus: std::sync::Arc<dyn CommandBus>,
}

impl CommandDispatcher {
    /// Creates a new command dispatcher.
    pub fn new(
        clock: std::sync::Arc<dyn Clock>,
        id_generator: std::sync::Arc<dyn IdGenerator>,
        owner_resolver: std::sync::Arc<dyn DeviceOwnerResolver>,
        command_bus: std::sync::Arc<dyn CommandBus>,
    ) -> Self {
        Self {
            clock,
            id_generator,
            owner_resolver,
            command_bus,
        }
    }

    /// Dispatches the command for a pending operation.
    pub async fn dispatch(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        tenant_id: TenantId,
        operation_id: OperationId,
    ) -> crate::Result<OperationDto> {
        let mut operation = uow
            .operation_repository()
            .get(tenant_id, operation_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "operation",
                    operation_id.to_string(),
                ))
            })?;

        if operation.is_terminal() || operation.status() == cheetah_domain::OperationStatus::Running
        {
            return Ok(OperationDto::from(&operation));
        }

        // Transition to Running and commit before external I/O.
        let event = operation
            .start(self.clock.as_ref())
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

        // Resolve owner and dispatch outside the unit of work transaction.
        match self
            .owner_resolver
            .resolve(tenant_id, operation.device_id())
            .await?
        {
            None => {
                let event = operation
                    .complete(
                        OperationResult::failure("NO_OWNER", "no owner resolved"),
                        self.clock.as_ref(),
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
            }
            Some(owner) if owner.owner_epoch != operation.expected_owner_epoch() => {
                let event = operation
                    .complete(
                        OperationResult::failure("STALE_OWNER", "stale owner epoch"),
                        self.clock.as_ref(),
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
            }
            Some(_) => {
                if let Err(e) = self.command_bus.send(operation.command()).await {
                    let event = operation
                        .complete(
                            OperationResult::failure("COMMAND_BUS", e.to_string()),
                            self.clock.as_ref(),
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
                }
            }
        }

        Ok(OperationDto::from(&operation))
    }
}

impl std::fmt::Debug for CommandDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandDispatcher").finish_non_exhaustive()
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

fn operation_resource_ref(tenant_id: TenantId, operation_id: OperationId) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Operation,
        id: ResourceId::Operation(operation_id),
    }
}

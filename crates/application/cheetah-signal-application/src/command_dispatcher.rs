//! Command dispatcher.

use crate::dto::OperationDto;
use cheetah_domain::{
    CommandBus, DeviceOwnerResolver, DispatchAttempt, DispatchAttemptStatus, DomainError,
    DomainEvent, OperationResult, OperationStatus, UnitOfWork,
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

    /// Dispatches the command for a pending or running operation.
    ///
    /// The dispatcher is idempotent: a terminal or already-sent operation is
    /// returned without re-dispatching. If the process crashed after the
    /// operation transitioned to `Running` but before `command_bus.send`
    /// succeeded, the next invocation will re-use the pending dispatch attempt
    /// and try to send again.
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

        if operation.is_terminal() {
            return Ok(OperationDto::from(&operation));
        }

        let step_id = operation
            .command()
            .step_id()
            .unwrap_or_else(|| operation.command().command_id());

        // A running operation whose command has already been sent should not be
        // re-dispatched. If it is running but has no sent/acked attempt, a
        // previous dispatch crashed before the command reached the bus.
        if operation.status() == OperationStatus::Running
            && has_sent_or_acked_attempt(&operation, step_id)
        {
            return Ok(OperationDto::from(&operation));
        }

        // Transition from Pending to Running and commit before external I/O.
        if operation.status() == OperationStatus::Pending {
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
        }

        // Re-use an existing pending attempt from a previous crash window, or
        // record a new one. Do not mark it sent until command_bus.send succeeds.
        let attempt_id = match find_pending_attempt_id(&operation, step_id) {
            Some(id) => id,
            None => {
                let id = self.id_generator.generate_message_id();
                let attempt = DispatchAttempt::new(id);
                operation
                    .record_dispatch_attempt(step_id, attempt, self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                id
            }
        };
        self.save_operation(uow, &operation).await?;

        // Resolve owner and dispatch outside the unit of work transaction.
        match self
            .owner_resolver
            .resolve(tenant_id, operation.device_id())
            .await
        {
            Ok(Some(owner)) if owner.owner_epoch == operation.expected_owner_epoch() => {
                match self.command_bus.send(operation.command()).await {
                    Ok(()) => {
                        operation
                            .mark_dispatch_attempt_sent(step_id, attempt_id, self.clock.as_ref())
                            .map_err(crate::SignalError::from)?;
                        self.save_operation(uow, &operation).await?;
                    }
                    Err(e) => {
                        operation
                            .mark_dispatch_attempt_nacked(
                                step_id,
                                attempt_id,
                                cheetah_domain::OperationError::new("COMMAND_BUS", e.to_string()),
                                self.clock.as_ref(),
                            )
                            .map_err(crate::SignalError::from)?;
                        self.save_operation(uow, &operation).await?;
                        self.complete_operation(
                            context,
                            uow,
                            tenant_id,
                            &mut operation,
                            "COMMAND_BUS",
                            e.to_string(),
                        )
                        .await?;
                    }
                }
            }
            Ok(Some(_)) => {
                operation
                    .mark_dispatch_attempt_nacked(
                        step_id,
                        attempt_id,
                        cheetah_domain::OperationError::new("STALE_OWNER", "stale owner epoch"),
                        self.clock.as_ref(),
                    )
                    .map_err(crate::SignalError::from)?;
                self.save_operation(uow, &operation).await?;
                self.complete_operation(
                    context,
                    uow,
                    tenant_id,
                    &mut operation,
                    "STALE_OWNER",
                    "stale owner epoch",
                )
                .await?;
            }
            Ok(None) => {
                operation
                    .mark_dispatch_attempt_nacked(
                        step_id,
                        attempt_id,
                        cheetah_domain::OperationError::new("NO_OWNER", "no owner resolved"),
                        self.clock.as_ref(),
                    )
                    .map_err(crate::SignalError::from)?;
                self.save_operation(uow, &operation).await?;
                self.complete_operation(
                    context,
                    uow,
                    tenant_id,
                    &mut operation,
                    "NO_OWNER",
                    "no owner resolved",
                )
                .await?;
            }
            Err(e) => {
                operation
                    .mark_dispatch_attempt_nacked(
                        step_id,
                        attempt_id,
                        cheetah_domain::OperationError::new("RESOLVE_ERROR", e.to_string()),
                        self.clock.as_ref(),
                    )
                    .map_err(crate::SignalError::from)?;
                self.save_operation(uow, &operation).await?;
                self.complete_operation(
                    context,
                    uow,
                    tenant_id,
                    &mut operation,
                    "RESOLVE_ERROR",
                    e.to_string(),
                )
                .await?;
            }
        }

        Ok(OperationDto::from(&operation))
    }

    async fn save_operation(
        &self,
        uow: &mut dyn UnitOfWork,
        operation: &cheetah_domain::Operation,
    ) -> crate::Result<()> {
        uow.operation_repository().save(operation).await?;
        uow.commit().await?;
        Ok(())
    }

    async fn complete_operation(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        tenant_id: TenantId,
        operation: &mut cheetah_domain::Operation,
        code: &str,
        message: impl Into<String>,
    ) -> crate::Result<()> {
        let event = operation
            .complete(OperationResult::failure(code, message), self.clock.as_ref())
            .map_err(crate::SignalError::from)?;
        uow.operation_repository().save(operation).await?;
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
        Ok(())
    }
}

impl std::fmt::Debug for CommandDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandDispatcher").finish_non_exhaustive()
    }
}

fn has_sent_or_acked_attempt(
    operation: &cheetah_domain::Operation,
    step_id: cheetah_signal_types::MessageId,
) -> bool {
    operation
        .steps()
        .iter()
        .find(|s| s.step_id() == step_id)
        .map(|s| {
            s.attempts().iter().any(|a| {
                matches!(
                    a.status(),
                    DispatchAttemptStatus::Sent | DispatchAttemptStatus::Acked
                )
            })
        })
        .unwrap_or(false)
}

fn find_pending_attempt_id(
    operation: &cheetah_domain::Operation,
    step_id: cheetah_signal_types::MessageId,
) -> Option<cheetah_signal_types::MessageId> {
    operation
        .steps()
        .iter()
        .find(|s| s.step_id() == step_id)
        .and_then(|s| {
            s.attempts()
                .iter()
                .rev()
                .find(|a| a.status() == DispatchAttemptStatus::Pending)
                .map(|a| a.attempt_id())
        })
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

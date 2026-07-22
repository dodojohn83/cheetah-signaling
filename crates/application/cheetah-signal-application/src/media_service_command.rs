//! Media command dispatch helpers for `MediaService`.

use crate::dto::{MediaSessionDto, OperationDto};
use crate::media_service::*;
use cheetah_domain::{
    CommandPayload, DomainError, MediaBindingError, MediaBindingState, MediaNodeCommand,
    MediaNodeCommandResult, MediaSessionError, MediaSessionState, OperationResult, UnitOfWork,
};
use cheetah_signal_types::{
    Deadline, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId, OperationId,
    OwnerEpoch, RequestContext,
};

impl MediaService {
    /// Loads the operation, starts it, commits, then executes the media-node command.
    /// Callers are responsible for applying the result to the aggregates.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_media_command(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        operation_id: OperationId,
        media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        media_node_id: NodeId,
        media_node_instance_epoch: MediaNodeInstanceEpoch,
        contract_version: u32,
        owner_epoch: OwnerEpoch,
        deadline: Option<Deadline>,
        idempotency_key: String,
        payload: CommandPayload,
    ) -> crate::Result<MediaNodeCommandResult> {
        let mut operation = uow
            .operation_repository()
            .get(context.tenant_id, operation_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "operation",
                    operation_id.to_string(),
                ))
            })?;

        let op_started_event = operation
            .start(self.clock.as_ref())
            .map_err(crate::SignalError::from)?;
        uow.operation_repository().save(&operation).await?;
        uow.outbox()
            .append(wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                context.tenant_id,
                operation_resource_ref(context.tenant_id, operation.operation_id()),
                operation.revision().0,
                op_started_event,
            ))
            .await?;

        uow.commit().await?;

        let command = MediaNodeCommand {
            request_id: context.message_id.to_string(),
            tenant_id: context.tenant_id,
            media_session_id,
            media_binding_id,
            media_node_id,
            media_node_instance_epoch,
            operation_id,
            owner_epoch,
            source_node_id: self.source_node_id,
            deadline,
            idempotency_key,
            contract_version,
            payload,
        };

        match self.media_port.execute(command, self.clock.as_ref()).await {
            Ok(r) => Ok(r),
            // Transport-level failures are converted to a command failure so the
            // dispatch methods can transition aggregates to terminal states and
            // release the scheduler reservation.
            Err(e) => Ok(domain_error_to_command_failure(e)),
        }
    }

    /// Dispatches a stop command to the media node and completes the stop lifecycle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn dispatch_stop_command(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        operation_id: OperationId,
        media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        media_node_id: NodeId,
        media_node_instance_epoch: MediaNodeInstanceEpoch,
        contract_version: u32,
        owner_epoch: OwnerEpoch,
        deadline: Option<Deadline>,
        idempotency_key: String,
    ) -> crate::Result<MediaSessionDto> {
        let payload = CommandPayload::StopMediaSession { media_session_id };
        let result = self
            .execute_media_command(
                context,
                uow,
                operation_id,
                media_session_id,
                media_binding_id,
                media_node_id,
                media_node_instance_epoch,
                contract_version,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )
            .await?;

        let mut operation = uow
            .operation_repository()
            .get(context.tenant_id, operation_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "operation",
                    operation_id.to_string(),
                ))
            })?;
        let mut session = uow
            .media_session_repository()
            .get(context.tenant_id, media_session_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "media session",
                    media_session_id.to_string(),
                ))
            })?;
        let mut binding = uow
            .media_binding_repository()
            .get(context.tenant_id, media_binding_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "media binding",
                    media_binding_id.to_string(),
                ))
            })?;

        match result {
            MediaNodeCommandResult::Completed => {
                // Stop the session, handling whatever intermediate state it is in.
                if session.state() == MediaSessionState::Active {
                    let ev = session
                        .stopping(self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_session_resource_ref(
                                context.tenant_id,
                                session.media_session_id(),
                            ),
                            session.revision().0,
                            ev,
                        ))
                        .await?;
                    uow.media_session_repository().save(&session).await?;
                }
                if !session.is_terminal() {
                    let ev = session
                        .stopped(self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_session_resource_ref(
                                context.tenant_id,
                                session.media_session_id(),
                            ),
                            session.revision().0,
                            ev,
                        ))
                        .await?;
                    uow.media_session_repository().save(&session).await?;
                }

                if binding.state() == MediaBindingState::Active
                    || binding.state() == MediaBindingState::Reserved
                {
                    let ev = binding
                        .release(self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_binding_resource_ref(
                                context.tenant_id,
                                binding.media_binding_id(),
                            ),
                            binding.revision().0,
                            ev,
                        ))
                        .await?;
                    uow.media_binding_repository().save(&binding).await?;
                }
                if !binding.is_terminal() {
                    let ev = binding
                        .released(self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_binding_resource_ref(
                                context.tenant_id,
                                binding.media_binding_id(),
                            ),
                            binding.revision().0,
                            ev,
                        ))
                        .await?;
                    uow.media_binding_repository().save(&binding).await?;
                }

                let op_event = operation
                    .complete(OperationResult::success(), self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;

                uow.operation_repository().save(&operation).await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        operation_resource_ref(context.tenant_id, operation.operation_id()),
                        operation.revision().0,
                        op_event,
                    ))
                    .await?;
                uow.commit().await?;

                if let Err(e) = self
                    .media_port
                    .release(context.tenant_id, media_binding_id, self.clock.as_ref())
                    .await
                {
                    tracing::warn!("failed to release media binding after stop: {e}");
                }

                Ok(MediaSessionDto::from(&session))
            }
            MediaNodeCommandResult::Accepted => {
                // The media node accepted the stop asynchronously. Record the
                // stop intent durably so a lost completion still converges to a
                // terminal state during reconciliation.
                if !session.is_terminal() && session.state() != MediaSessionState::Stopping {
                    let ev = session
                        .stop(self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_session_resource_ref(
                                context.tenant_id,
                                session.media_session_id(),
                            ),
                            session.revision().0,
                            ev,
                        ))
                        .await?;
                    uow.media_session_repository().save(&session).await?;
                }
                if !binding.is_terminal() && binding.state() != MediaBindingState::Releasing {
                    let ev = binding
                        .release(self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_binding_resource_ref(
                                context.tenant_id,
                                binding.media_binding_id(),
                            ),
                            binding.revision().0,
                            ev,
                        ))
                        .await?;
                    uow.media_binding_repository().save(&binding).await?;
                }

                uow.commit().await?;
                Ok(MediaSessionDto::from(&session))
            }
            // The media node could not confirm whether the device-side teardown
            // completed. Record the stop intent durably (like an accepted stop),
            // perform bounded media cleanup by releasing the reservation, and
            // leave the operation running so the reconciler confirms the outcome.
            MediaNodeCommandResult::UnknownOutcome { code, message } => {
                tracing::warn!(
                    tenant_id = %context.tenant_id,
                    media_binding_id = %media_binding_id,
                    code = %code,
                    "stop command returned unknown outcome; deferring to reconciler: {message}"
                );
                if !session.is_terminal() && session.state() != MediaSessionState::Stopping {
                    let ev = session
                        .stop(self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_session_resource_ref(
                                context.tenant_id,
                                session.media_session_id(),
                            ),
                            session.revision().0,
                            ev,
                        ))
                        .await?;
                    uow.media_session_repository().save(&session).await?;
                }
                if !binding.is_terminal() && binding.state() != MediaBindingState::Releasing {
                    let ev = binding
                        .release(self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_binding_resource_ref(
                                context.tenant_id,
                                binding.media_binding_id(),
                            ),
                            binding.revision().0,
                            ev,
                        ))
                        .await?;
                    uow.media_binding_repository().save(&binding).await?;
                }

                uow.commit().await?;

                // Bounded media cleanup: release the reservation using the
                // original binding (node instance epoch + idempotency key).
                if let Err(e) = self
                    .media_port
                    .release(context.tenant_id, media_binding_id, self.clock.as_ref())
                    .await
                {
                    tracing::warn!(
                        "failed to release media binding after unknown stop outcome: {e}"
                    );
                }
                Ok(MediaSessionDto::from(&session))
            }
            MediaNodeCommandResult::Failed { code, message } => {
                if !session.is_terminal() {
                    let session_event = session
                        .failed(MediaSessionError::new(&code, &message), self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_session_resource_ref(
                                context.tenant_id,
                                session.media_session_id(),
                            ),
                            session.revision().0,
                            session_event,
                        ))
                        .await?;
                }
                if !binding.is_terminal() {
                    let binding_event = binding
                        .failed(MediaBindingError::new(&code, &message), self.clock.as_ref())
                        .map_err(crate::SignalError::from)?;
                    uow.outbox()
                        .append(wrap_event(
                            self.id_generator.as_ref(),
                            self.clock.as_ref(),
                            context,
                            context.tenant_id,
                            media_binding_resource_ref(
                                context.tenant_id,
                                binding.media_binding_id(),
                            ),
                            binding.revision().0,
                            binding_event,
                        ))
                        .await?;
                }
                let op_event = operation
                    .complete(
                        OperationResult::failure(&code, &message),
                        self.clock.as_ref(),
                    )
                    .map_err(crate::SignalError::from)?;

                uow.media_session_repository().save(&session).await?;
                uow.media_binding_repository().save(&binding).await?;
                uow.operation_repository().save(&operation).await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        operation_resource_ref(context.tenant_id, operation.operation_id()),
                        operation.revision().0,
                        op_event,
                    ))
                    .await?;
                uow.commit().await?;

                if let Err(e) = self
                    .media_port
                    .release(context.tenant_id, media_binding_id, self.clock.as_ref())
                    .await
                {
                    tracing::warn!("failed to release media binding after stop failure: {e}");
                }

                Err(crate::SignalError::from(DomainError::unavailable(format!(
                    "media node stop failed: {code}: {message}"
                ))))
            }
        }
    }

    /// Dispatches a playback control command to the media node.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn dispatch_control_command(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        operation_id: OperationId,
        media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        media_node_id: NodeId,
        media_node_instance_epoch: MediaNodeInstanceEpoch,
        contract_version: u32,
        owner_epoch: OwnerEpoch,
        deadline: Option<Deadline>,
        idempotency_key: String,
        payload: CommandPayload,
    ) -> crate::Result<OperationDto> {
        let result = self
            .execute_media_command(
                context,
                uow,
                operation_id,
                media_session_id,
                media_binding_id,
                media_node_id,
                media_node_instance_epoch,
                contract_version,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )
            .await?;

        let mut operation = uow
            .operation_repository()
            .get(context.tenant_id, operation_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "operation",
                    operation_id.to_string(),
                ))
            })?;

        match result {
            MediaNodeCommandResult::Completed => {
                let op_event = operation
                    .complete(OperationResult::success(), self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                uow.operation_repository().save(&operation).await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        operation_resource_ref(context.tenant_id, operation.operation_id()),
                        operation.revision().0,
                        op_event,
                    ))
                    .await?;
                uow.commit().await?;
                Ok(OperationDto::from(&operation))
            }
            MediaNodeCommandResult::Accepted => {
                uow.commit().await?;
                Ok(OperationDto::from(&operation))
            }
            // The media node could not confirm whether the control command took
            // effect. Leave the operation running for the reconciler rather than
            // completing it as success or failure.
            MediaNodeCommandResult::UnknownOutcome { code, message } => {
                tracing::warn!(
                    tenant_id = %context.tenant_id,
                    operation_id = %operation.operation_id(),
                    code = %code,
                    "control command returned unknown outcome; deferring to reconciler: {message}"
                );
                uow.commit().await?;
                Ok(OperationDto::from(&operation))
            }
            MediaNodeCommandResult::Failed { code, message } => {
                let op_event = operation
                    .complete(
                        OperationResult::failure(&code, &message),
                        self.clock.as_ref(),
                    )
                    .map_err(crate::SignalError::from)?;
                uow.operation_repository().save(&operation).await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        operation_resource_ref(context.tenant_id, operation.operation_id()),
                        operation.revision().0,
                        op_event,
                    ))
                    .await?;
                uow.commit().await?;
                Err(crate::SignalError::from(DomainError::unavailable(format!(
                    "media node control failed: {code}: {message}"
                ))))
            }
        }
    }
}

/// Maps a domain error from the media port into a `Failed` command result.
/// This lets the dispatch methods apply the same terminal-state handling as
/// a business-level failure from the media node.
fn domain_error_to_command_failure(e: DomainError) -> MediaNodeCommandResult {
    let code = match &e {
        DomainError::Unavailable { .. } => "unavailable",
        DomainError::InvalidArgument { .. } => "invalid_argument",
        DomainError::NotFound { .. } => "not_found",
        DomainError::ConcurrentModification { .. } => "concurrency_conflict",
        DomainError::StaleOwner { .. } => "stale_owner",
        _ => "media_command_failed",
    };
    MediaNodeCommandResult::Failed {
        code: code.to_string(),
        message: e.to_string(),
    }
}

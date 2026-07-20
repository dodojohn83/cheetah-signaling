//! Media-node callback handler for `MediaService`.

use crate::dto::MediaSessionDto;
use crate::media_service::*;
use cheetah_domain::{
    DomainError, MediaBindingError, MediaBindingState, MediaNodeCallback, MediaNodeCallbackKind,
    MediaSessionError, MediaSessionState, OperationResult, UnitOfWork,
};
use cheetah_signal_types::RequestContext;

impl MediaService {
    /// Handles an asynchronous event from a media node, validating fencing
    /// fields before updating the binding, session and operation.
    ///
    /// This helper commits `uow`; callers that need cursor/inbox atomicity should
    /// use [`MediaService::apply_media_event`] and commit themselves.
    pub async fn handle_media_event(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        callback: MediaNodeCallback,
    ) -> crate::Result<MediaSessionDto> {
        let dto = self.apply_media_event(context, uow, callback).await?;
        uow.commit().await?;
        Ok(dto)
    }

    /// Applies an asynchronous event from a media node without committing `uow`.
    ///
    /// The caller is responsible for de-duplication, cursor persistence and
    /// transaction commit. Errors are returned as [`DomainError`] so the
    /// scheduler's event consumer can decide whether to retry.
    pub(crate) async fn apply_media_event(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        callback: MediaNodeCallback,
    ) -> Result<MediaSessionDto, DomainError> {
        let mut binding = uow
            .media_binding_repository()
            .get(context.tenant_id, callback.media_binding_id)
            .await?
            .ok_or_else(|| {
                DomainError::not_found("media binding", callback.media_binding_id.to_string())
            })?;

        if binding.media_node_id() != callback.media_node_id {
            return Err(DomainError::invalid_argument("media node id mismatch"));
        }
        if binding.media_node_instance_epoch() != callback.media_node_instance_epoch {
            return Err(DomainError::invalid_argument(
                "media node instance epoch mismatch",
            ));
        }
        if binding.owner_epoch() != callback.owner_epoch {
            return Err(DomainError::invalid_argument("owner epoch mismatch"));
        }
        if binding.revision().0 != callback.binding_revision.0 {
            return Err(DomainError::ConcurrentModification {
                expected: callback.binding_revision.0,
                found: binding.revision().0,
            });
        }

        let mut session = uow
            .media_session_repository()
            .get(context.tenant_id, callback.media_session_id)
            .await?
            .ok_or_else(|| {
                DomainError::not_found("media session", callback.media_session_id.to_string())
            })?;

        if session.revision().0 != callback.session_revision.0 {
            return Err(DomainError::ConcurrentModification {
                expected: callback.session_revision.0,
                found: session.revision().0,
            });
        }

        let mut operation: Option<cheetah_domain::Operation> = if let Some(operation_id) =
            callback.operation_id
        {
            Some(
                uow.operation_repository()
                    .get(context.tenant_id, operation_id)
                    .await?
                    .ok_or_else(|| DomainError::not_found("operation", operation_id.to_string()))?,
            )
        } else {
            None
        };

        // Ignore late callbacks for operations that are already terminal.
        if operation.as_ref().is_some_and(|o| o.is_terminal()) {
            return Ok(MediaSessionDto::from(&session));
        }

        let session_state_before = session.state();
        let binding_state_before = binding.state();
        let operation_status_before = operation.as_ref().map(|o| o.status());

        match callback.kind {
            MediaNodeCallbackKind::Started => {
                apply_started(
                    self,
                    context,
                    uow,
                    &mut session,
                    &mut binding,
                    &mut operation,
                )
                .await?;
            }
            MediaNodeCallbackKind::Stopped { reason } => {
                apply_stopped(
                    self,
                    context,
                    uow,
                    &mut session,
                    &mut binding,
                    &mut operation,
                    &reason,
                )
                .await?;
            }
            MediaNodeCallbackKind::Failed { code, message } => {
                apply_failed(
                    self,
                    context,
                    uow,
                    &mut session,
                    &mut binding,
                    &mut operation,
                    &code,
                    &message,
                )
                .await?;
            }
        }

        if session.state() != session_state_before {
            uow.media_session_repository().save(&session).await?;
        }
        if binding.state() != binding_state_before {
            uow.media_binding_repository().save(&binding).await?;
        }
        if let Some(operation) = operation.as_ref() {
            let before = operation_status_before.unwrap_or(operation.status());
            if operation.status() != before {
                uow.operation_repository().save(operation).await?;
            }
        }

        Ok(MediaSessionDto::from(&session))
    }
}

#[async_trait::async_trait]
impl cheetah_domain::MediaEventHandler for MediaService {
    async fn handle_media_event(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        callback: MediaNodeCallback,
    ) -> Result<(), DomainError> {
        let _ = self.apply_media_event(context, uow, callback).await?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_started(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &mut cheetah_domain::MediaSession,
    binding: &mut cheetah_domain::MediaBinding,
    operation: &mut Option<cheetah_domain::Operation>,
) -> Result<(), DomainError> {
    if session.state() == MediaSessionState::Inviting {
        let ev = session.active(service.clock.as_ref())?;
        append_session_event(service, context, uow, session, ev).await?;
    }
    if binding.state() == MediaBindingState::Reserved {
        let ev = binding.activate(service.clock.as_ref())?;
        append_binding_event(service, context, uow, binding, ev).await?;
    }
    if let Some(operation) = operation.as_mut().filter(|o| !o.is_terminal()) {
        let ev = operation.complete(OperationResult::success(), service.clock.as_ref())?;
        append_operation_event(service, context, uow, operation, ev).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_stopped(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &mut cheetah_domain::MediaSession,
    binding: &mut cheetah_domain::MediaBinding,
    operation: &mut Option<cheetah_domain::Operation>,
    reason: &str,
) -> Result<(), DomainError> {
    if session.state() == MediaSessionState::Active {
        let ev = session.stopping(service.clock.as_ref())?;
        append_session_event(service, context, uow, session, ev).await?;
    }
    if !session.is_terminal() {
        let ev = session.stopped(service.clock.as_ref())?;
        append_session_event(service, context, uow, session, ev).await?;
    }

    if binding.state() == MediaBindingState::Active {
        let ev = binding.release(service.clock.as_ref())?;
        append_binding_event(service, context, uow, binding, ev).await?;
    }
    if binding.state() == MediaBindingState::Releasing {
        let ev = binding.released(service.clock.as_ref())?;
        append_binding_event(service, context, uow, binding, ev).await?;
    }

    if let Some(operation) = operation.as_mut().filter(|o| !o.is_terminal()) {
        let result = if reason.is_empty() {
            OperationResult::success()
        } else {
            OperationResult::failure("stopped", reason)
        };
        let ev = operation.complete(result, service.clock.as_ref())?;
        append_operation_event(service, context, uow, operation, ev).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_failed(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &mut cheetah_domain::MediaSession,
    binding: &mut cheetah_domain::MediaBinding,
    operation: &mut Option<cheetah_domain::Operation>,
    code: &str,
    message: &str,
) -> Result<(), DomainError> {
    if !session.is_terminal() {
        let ev = session.failed(
            MediaSessionError::new(code, message),
            service.clock.as_ref(),
        )?;
        append_session_event(service, context, uow, session, ev).await?;
    }
    if !binding.is_terminal() {
        let ev = binding.failed(
            MediaBindingError::new(code, message),
            service.clock.as_ref(),
        )?;
        append_binding_event(service, context, uow, binding, ev).await?;
    }
    if let Some(operation) = operation.as_mut().filter(|o| !o.is_terminal()) {
        let ev = operation.complete(
            OperationResult::failure(code, message),
            service.clock.as_ref(),
        )?;
        append_operation_event(service, context, uow, operation, ev).await?;
    }
    Ok(())
}

async fn append_session_event(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &cheetah_domain::MediaSession,
    payload: cheetah_domain::DomainEvent,
) -> Result<(), DomainError> {
    uow.outbox()
        .append(wrap_event(
            service.id_generator.as_ref(),
            service.clock.as_ref(),
            context,
            context.tenant_id,
            media_session_resource_ref(context.tenant_id, session.media_session_id()),
            session.revision().0,
            payload,
        ))
        .await
}

async fn append_binding_event(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    binding: &cheetah_domain::MediaBinding,
    payload: cheetah_domain::DomainEvent,
) -> Result<(), DomainError> {
    uow.outbox()
        .append(wrap_event(
            service.id_generator.as_ref(),
            service.clock.as_ref(),
            context,
            context.tenant_id,
            media_binding_resource_ref(context.tenant_id, binding.media_binding_id()),
            binding.revision().0,
            payload,
        ))
        .await
}

async fn append_operation_event(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    operation: &cheetah_domain::Operation,
    payload: cheetah_domain::DomainEvent,
) -> Result<(), DomainError> {
    uow.outbox()
        .append(wrap_event(
            service.id_generator.as_ref(),
            service.clock.as_ref(),
            context,
            context.tenant_id,
            operation_resource_ref(context.tenant_id, operation.operation_id()),
            operation.revision().0,
            payload,
        ))
        .await
}

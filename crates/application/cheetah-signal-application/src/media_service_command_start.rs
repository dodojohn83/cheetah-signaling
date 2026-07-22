//! Media start-command dispatch for `MediaService`.
//!
//! Split out from `media_service_command` to keep each source file within the
//! repository's file-length limit; the start path applies a media-node command
//! result to the operation, media session and media binding aggregates.

use crate::dto::MediaSessionDto;
use crate::media_service::*;
use cheetah_domain::{
    CommandPayload, DomainError, MediaBindingError, MediaNodeCommandResult, MediaSessionError,
    OperationResult, UnitOfWork,
};
use cheetah_signal_types::{
    Deadline, MediaBindingId, MediaSessionId, OperationId, OwnerEpoch, RequestContext,
};

impl MediaService {
    /// Sends a start command to the media node and applies the result to the
    /// operation, media session and media binding.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn dispatch_media_command(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        operation_id: OperationId,
        media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        reservation: &cheetah_domain::MediaReservation,
        owner_epoch: OwnerEpoch,
        deadline: Option<Deadline>,
        idempotency_key: String,
        payload: CommandPayload,
    ) -> crate::Result<MediaSessionDto> {
        let result = self
            .execute_media_command(
                context,
                uow,
                operation_id,
                media_session_id,
                media_binding_id,
                reservation.media_node_id,
                reservation.media_node_instance_epoch,
                reservation.contract_version,
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
                // A synchronous start still goes through the intermediate inviting
                // state before becoming active.
                let session_inviting_event = session
                    .inviting(self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                let session_inviting_revision = session.revision().0;
                uow.media_session_repository().save(&session).await?;
                let session_active_event = session
                    .active(self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                let session_active_revision = session.revision().0;
                let binding_event = binding
                    .activate(self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                let op_event = operation
                    .complete(OperationResult::success(), self.clock.as_ref())
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
                        media_session_resource_ref(context.tenant_id, session.media_session_id()),
                        session_inviting_revision,
                        session_inviting_event,
                    ))
                    .await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        media_session_resource_ref(context.tenant_id, session.media_session_id()),
                        session_active_revision,
                        session_active_event,
                    ))
                    .await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        media_binding_resource_ref(context.tenant_id, binding.media_binding_id()),
                        binding.revision().0,
                        binding_event,
                    ))
                    .await?;
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
            }
            MediaNodeCommandResult::Accepted => {
                let session_event = session
                    .inviting(self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                let binding_event = binding
                    .activate(self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;

                uow.media_session_repository().save(&session).await?;
                uow.media_binding_repository().save(&binding).await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        media_session_resource_ref(context.tenant_id, session.media_session_id()),
                        session.revision().0,
                        session_event,
                    ))
                    .await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        media_binding_resource_ref(context.tenant_id, binding.media_binding_id()),
                        binding.revision().0,
                        binding_event,
                    ))
                    .await?;
                uow.commit().await?;
            }
            // The media node could not confirm whether the start took effect.
            // Do not fail terminally (which could orphan a started sender) and do
            // not release the reservation; drive the session to Inviting and
            // activate the binding like an accepted start, and leave the operation
            // running so the reconciler resolves the real state by querying the node.
            MediaNodeCommandResult::UnknownOutcome { code, message } => {
                tracing::warn!(
                    tenant_id = %context.tenant_id,
                    media_session_id = %media_session_id,
                    media_binding_id = %media_binding_id,
                    code = %code,
                    "start command returned unknown outcome; deferring to reconciler: {message}"
                );
                let session_event = session
                    .inviting(self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                let binding_event = binding
                    .activate(self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;

                uow.media_session_repository().save(&session).await?;
                uow.media_binding_repository().save(&binding).await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        media_session_resource_ref(context.tenant_id, session.media_session_id()),
                        session.revision().0,
                        session_event,
                    ))
                    .await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        media_binding_resource_ref(context.tenant_id, binding.media_binding_id()),
                        binding.revision().0,
                        binding_event,
                    ))
                    .await?;
                uow.commit().await?;
            }
            MediaNodeCommandResult::Failed { code, message } => {
                let session_event = session
                    .failed(MediaSessionError::new(&code, &message), self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                let binding_event = binding
                    .failed(MediaBindingError::new(&code, &message), self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
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
                        media_session_resource_ref(context.tenant_id, session.media_session_id()),
                        session.revision().0,
                        session_event,
                    ))
                    .await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        context.tenant_id,
                        media_binding_resource_ref(context.tenant_id, binding.media_binding_id()),
                        binding.revision().0,
                        binding_event,
                    ))
                    .await?;
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

                // Bilateral compensation: release the media-node sender/receiver
                // reservation. The media node reports failure only after tearing
                // down any dialog it opened, so releasing the reservation
                // completes the signaling side of the compensation.
                if let Err(e) = self
                    .media_port
                    .release(context.tenant_id, media_binding_id, self.clock.as_ref())
                    .await
                {
                    tracing::warn!("failed to release media binding after start failure: {e}");
                }

                return Err(crate::SignalError::from(DomainError::unavailable(format!(
                    "media node command failed: {code}: {message}"
                ))));
            }
        }

        Ok(MediaSessionDto::from(&session))
    }
}

//! Media session reconnection dispatch helper for `MediaService`.
//!
//! Used by the reconciler to migrate an existing `MediaSession` to a new media
//! node when the current binding is lost, the node is draining, or the node
//! instance has changed.

use crate::dto::MediaSessionDto;
use crate::media_service::*;
use cheetah_domain::{
    CommandPayload, DomainError, MediaBinding, MediaBindingError, MediaNodeCommandResult,
    MediaPurpose, MediaReservation, MediaSession, MediaSessionError, Operation, OperationResult,
    UnitOfWork,
};
use cheetah_signal_types::{
    ChannelId, Deadline, DeviceId, DurationMs, MediaBindingId, RequestContext,
};

impl MediaService {
    /// Reconnects an existing media session to a new media node.
    ///
    /// The old binding is marked failed, the session generation is bumped, a
    /// new binding is reserved, and a start command is dispatched to the new
    /// node. This is used by the reconciler when a session is active but its
    /// media node is missing, draining, or has lost the resource.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn dispatch_reconnect_command(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        session: &mut MediaSession,
        old_binding: &mut MediaBinding,
        device_id: DeviceId,
        channel_id: ChannelId,
        media_binding_id: MediaBindingId,
        reservation: &MediaReservation,
    ) -> crate::Result<MediaSessionDto> {
        let tenant_id = context.tenant_id;
        let media_session_id = session.media_session_id();
        let idempotency_key = format!(
            "reconnect-{}-{}-{}",
            media_session_id,
            session.generation(),
            self.id_generator.generate_message_id()
        );
        // Use a fresh deadline for the reconnect command so that an elapsed
        // original request deadline does not prevent migration.
        let deadline = Deadline::from_now(self.clock.now_wall(), DurationMs::from_seconds(30));
        let owner_epoch = session.owner_epoch();

        // Mark the old binding as failed so it is never resurrected.
        if !old_binding.is_terminal() {
            let ev = old_binding
                .failed(
                    MediaBindingError::media_node_unavailable(),
                    self.clock.as_ref(),
                )
                .map_err(crate::SignalError::from)?;
            uow.media_binding_repository().save(old_binding).await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_binding_resource_ref(tenant_id, old_binding.media_binding_id()),
                    old_binding.revision().0,
                    ev,
                ))
                .await?;
        }

        let target = media_session_resource_ref(tenant_id, media_session_id);
        let payload = match session.purpose() {
            MediaPurpose::Live => CommandPayload::StartLive {
                media_session_id,
                channel_id,
                media_node_id: reservation.media_node_id,
                purpose: MediaPurpose::Live,
            },
            MediaPurpose::Playback => {
                let start_time = session
                    .playback_start_time()
                    .unwrap_or_else(|| self.clock.now_wall());
                let end_time = session.playback_end_time().unwrap_or(start_time);
                let scale = session.playback_scale().unwrap_or(1.0);
                CommandPayload::StartPlayback {
                    media_session_id,
                    channel_id,
                    media_node_id: reservation.media_node_id,
                    start_time,
                    end_time,
                    scale,
                }
            }
            MediaPurpose::Talk => CommandPayload::StartTalk {
                media_session_id,
                channel_id,
                media_node_id: reservation.media_node_id,
            },
            _ => {
                return Err(crate::SignalError::from(DomainError::invalid_argument(
                    "unknown media purpose",
                )));
            }
        };

        let (operation, op_created_event) = Operation::new(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            idempotency_key,
            device_id,
            target,
            payload,
            deadline,
            owner_epoch,
        )
        .map_err(crate::SignalError::from)?;
        let operation_id = operation.operation_id();

        let (mut new_binding, binding_event) = MediaBinding::new(
            self.clock.as_ref(),
            media_binding_id,
            media_session_id,
            tenant_id,
            channel_id,
            reservation.media_node_id,
            owner_epoch,
            reservation.media_node_instance_epoch,
        )
        .map_err(crate::SignalError::from)?;

        uow.operation_repository().save(&operation).await?;
        uow.media_binding_repository().save(&new_binding).await?;
        uow.outbox()
            .append(wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                tenant_id,
                operation_resource_ref(tenant_id, operation.operation_id()),
                operation.revision().0,
                op_created_event,
            ))
            .await?;
        uow.outbox()
            .append(wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                tenant_id,
                media_binding_resource_ref(tenant_id, new_binding.media_binding_id()),
                new_binding.revision().0,
                binding_event,
            ))
            .await?;

        uow.commit().await?;

        let result = match self
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
                operation.idempotency_scope().idempotency_key.clone(),
                operation.command().payload().clone(),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if let Err(e2) = self
                    .media_port
                    .release(tenant_id, media_binding_id, self.clock.as_ref())
                    .await
                {
                    tracing::warn!(
                        "failed to release scheduler reservation after reconnect dispatch error: {e2}"
                    );
                }
                return Err(e);
            }
        };

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

        match result {
            MediaNodeCommandResult::Completed | MediaNodeCommandResult::Accepted => {
                // Drive the session/binding to Active the same way the regular
                // reconcile loop does, then complete the operation. Any error
                // after this point must release the new scheduler reservation
                // because the binding has already been committed.
                let post_result: crate::Result<()> = async {
                    self.converge_active(context, uow, session, &mut new_binding)
                        .await?;

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

                    if operation.status() == cheetah_domain::OperationStatus::Running {
                        let op_event = operation
                            .complete(OperationResult::success(), self.clock.as_ref())
                            .map_err(crate::SignalError::from)?;
                        uow.operation_repository().save(&operation).await?;
                        uow.outbox()
                            .append(wrap_event(
                                self.id_generator.as_ref(),
                                self.clock.as_ref(),
                                context,
                                tenant_id,
                                operation_resource_ref(tenant_id, operation_id),
                                operation.revision().0,
                                op_event,
                            ))
                            .await?;
                    }
                    Ok(())
                }
                .await;

                if let Err(e) = post_result {
                    if let Err(e2) = self
                        .media_port
                        .release(tenant_id, media_binding_id, self.clock.as_ref())
                        .await
                    {
                        tracing::warn!(
                            "failed to release media binding after reconnect post-execute error: {e2}"
                        );
                    }
                    return Err(e);
                }
            }
            MediaNodeCommandResult::Failed { code, message } => {
                let ev = new_binding
                    .failed(MediaBindingError::new(&code, &message), self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                uow.media_binding_repository().save(&new_binding).await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        tenant_id,
                        media_binding_resource_ref(tenant_id, media_binding_id),
                        new_binding.revision().0,
                        ev,
                    ))
                    .await?;

                let ev = session
                    .failed(MediaSessionError::new(&code, &message), self.clock.as_ref())
                    .map_err(crate::SignalError::from)?;
                uow.media_session_repository().save(session).await?;
                uow.outbox()
                    .append(wrap_event(
                        self.id_generator.as_ref(),
                        self.clock.as_ref(),
                        context,
                        tenant_id,
                        media_session_resource_ref(tenant_id, media_session_id),
                        session.revision().0,
                        ev,
                    ))
                    .await?;

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
                        tenant_id,
                        operation_resource_ref(tenant_id, operation_id),
                        operation.revision().0,
                        op_event,
                    ))
                    .await?;

                if let Err(e) = self
                    .media_port
                    .release(tenant_id, media_binding_id, self.clock.as_ref())
                    .await
                {
                    tracing::warn!("failed to release media binding after reconnect failure: {e}");
                }

                return Err(crate::SignalError::from(DomainError::unavailable(format!(
                    "media node reconnect failed: {code}: {message}"
                ))));
            }
        }

        Ok(MediaSessionDto::from(&*session))
    }
}

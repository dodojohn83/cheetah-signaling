//! Media command dispatch helpers for `MediaService`.

use crate::dto::{MediaSessionDto, OperationDto};
use crate::media_service::*;
use cheetah_domain::{
    CommandPayload, DomainError, MediaBinding, MediaBindingError, MediaBindingState,
    MediaNodeCommand, MediaNodeCommandResult, MediaPurpose, MediaReservation, MediaSession,
    MediaSessionError, MediaSessionState, Operation, OperationResult, UnitOfWork,
};
use cheetah_signal_types::{
    ChannelId, Deadline, DeviceId, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId,
    OperationId, OwnerEpoch, RequestContext,
};

impl MediaService {
    /// Loads the operation, starts it, commits, then executes the media-node command.
    /// Callers are responsible for applying the result to the aggregates.
    #[allow(clippy::too_many_arguments)]
    async fn execute_media_command(
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

                if binding.state() == MediaBindingState::Active {
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
                if !session.is_terminal() {
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
        let deadline = session.deadline();
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
                operation.idempotency_scope().idempotency_key.clone(),
                operation.command().payload().clone(),
            )
            .await?;

        // Reload after execute_media_command committed.
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
                if session.state() != MediaSessionState::Active {
                    let ev = session
                        .active(self.clock.as_ref())
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
                }
                if new_binding.state() == MediaBindingState::Reserved {
                    let ev = new_binding
                        .activate(self.clock.as_ref())
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
                }
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
                uow.commit().await?;
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
                uow.commit().await?;

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

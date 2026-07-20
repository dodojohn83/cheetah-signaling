//! Media start operations.

use crate::dto::{MediaSessionDto, StartLiveRequest, StartPlaybackRequest, StartTalkRequest};
use crate::media_service::*;
use cheetah_domain::{
    CommandPayload, DomainError, IdempotencyScope, MediaBinding, MediaPurpose, MediaSession,
    MediaSessionDesiredState, Operation, UnitOfWork,
};
use cheetah_signal_types::{ChannelId, DeviceId, RequestContext, UtcTimestamp};

impl MediaService {
    /// Starts a live media session.
    #[allow(clippy::large_futures)]
    pub async fn start_live(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        request: StartLiveRequest,
    ) -> crate::Result<MediaSessionDto> {
        let tenant_id = context.tenant_id;
        let device_id = request.device_id.parse::<DeviceId>()?;
        let channel_id = request.channel_id.parse::<ChannelId>()?;

        let (device, channel) = self
            .ensure_device_and_channel_ready(uow, tenant_id, device_id, channel_id)
            .await?;

        let target = channel_resource_ref(tenant_id, channel_id);
        let scope = IdempotencyScope::new(
            tenant_id,
            context.principal.id.clone(),
            target,
            request.idempotency_key,
        )
        .map_err(crate::SignalError::from)?;

        if let Some(existing) = uow
            .media_session_repository()
            .get_by_idempotency(&scope)
            .await?
        {
            return Ok(MediaSessionDto::from(&existing));
        }

        let owner = self
            .owner_resolver
            .resolve(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::no_owner(device_id.to_string()))
            })?;

        let media_session_id = self.id_generator.generate_media_session_id();
        let media_binding_id = self.id_generator.generate_media_binding_id();
        let deadline = parse_deadline(request.deadline)?;
        let requirements = build_media_requirements(
            &device,
            &channel,
            MediaPurpose::Live,
            media_session_id,
            std::collections::BTreeMap::new(),
        );

        let reservation = self
            .media_port
            .reserve_live(
                tenant_id,
                device_id,
                channel_id,
                media_session_id,
                media_binding_id,
                MediaPurpose::Live,
                &requirements,
                self.clock.as_ref(),
            )
            .await?;

        let result = async {
            let (operation, op_event) = Operation::new(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                scope.idempotency_key.clone(),
                device_id,
                scope.target,
                CommandPayload::StartLive {
                    media_session_id,
                    channel_id,
                    media_node_id: reservation.media_node_id,
                    purpose: MediaPurpose::Live,
                },
                deadline,
                owner.owner_epoch,
            )
            .map_err(crate::SignalError::from)?;

            let (mut session, session_event) = MediaSession::new(
                self.clock.as_ref(),
                media_session_id,
                tenant_id,
                device_id,
                channel_id,
                MediaPurpose::Live,
                MediaSessionDesiredState::Active,
                owner.owner_epoch,
                operation.operation_id(),
                operation.idempotency_scope().clone(),
                deadline,
            )
            .map_err(crate::SignalError::from)?;

            let (binding, binding_event) = MediaBinding::new(
                self.clock.as_ref(),
                media_binding_id,
                media_session_id,
                tenant_id,
                channel_id,
                reservation.media_node_id,
                owner.owner_epoch,
                reservation.media_node_instance_epoch,
            )
            .map_err(crate::SignalError::from)?;

            uow.operation_repository().save(&operation).await?;
            uow.media_session_repository().save(&session).await?;
            uow.media_binding_repository().save(&binding).await?;

            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    operation_resource_ref(tenant_id, operation.operation_id()),
                    operation.revision().0,
                    op_event,
                ))
                .await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_session_resource_ref(tenant_id, session.media_session_id()),
                    session.revision().0,
                    session_event,
                ))
                .await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_binding_resource_ref(tenant_id, binding.media_binding_id()),
                    binding.revision().0,
                    binding_event,
                ))
                .await?;

            let allocating_event = session.allocating(self.clock.as_ref())?;
            uow.media_session_repository().save(&session).await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_session_resource_ref(tenant_id, session.media_session_id()),
                    session.revision().0,
                    allocating_event,
                ))
                .await?;

            uow.commit().await?;
            Ok((
                operation.operation_id(),
                session.media_session_id(),
                binding.media_binding_id(),
                reservation,
                owner.owner_epoch,
                deadline,
                scope.idempotency_key.clone(),
                operation.command().payload().clone(),
            ))
        }
        .await;

        let released = if result.is_err() {
            self.media_port
                .release(tenant_id, media_binding_id, self.clock.as_ref())
                .await
        } else {
            Ok(())
        };
        if let Err(e) = released {
            tracing::warn!("failed to release media reservation after failed start_live: {e}");
        }

        match result {
            Ok((
                operation_id,
                media_session_id,
                media_binding_id,
                reservation,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )) => {
                self.dispatch_media_command(
                    context,
                    uow,
                    operation_id,
                    media_session_id,
                    media_binding_id,
                    &reservation,
                    owner_epoch,
                    deadline,
                    idempotency_key,
                    payload,
                )
                .await
            }
            Err(e) => Err(e),
        }
    }

    /// Starts a playback session.
    #[allow(clippy::large_futures)]
    pub async fn start_playback(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        request: StartPlaybackRequest,
    ) -> crate::Result<MediaSessionDto> {
        let tenant_id = context.tenant_id;
        let device_id = request.device_id.parse::<DeviceId>()?;
        let channel_id = request.channel_id.parse::<ChannelId>()?;
        let start_time = request.start_time.parse::<UtcTimestamp>()?;
        let end_time = request.end_time.parse::<UtcTimestamp>()?;

        let (device, channel) = self
            .ensure_device_and_channel_ready(uow, tenant_id, device_id, channel_id)
            .await?;

        let target = channel_resource_ref(tenant_id, channel_id);
        let scope = IdempotencyScope::new(
            tenant_id,
            context.principal.id.clone(),
            target,
            request.idempotency_key,
        )
        .map_err(crate::SignalError::from)?;

        if let Some(existing) = uow
            .media_session_repository()
            .get_by_idempotency(&scope)
            .await?
        {
            return Ok(MediaSessionDto::from(&existing));
        }

        let owner = self
            .owner_resolver
            .resolve(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::no_owner(device_id.to_string()))
            })?;

        let media_session_id = self.id_generator.generate_media_session_id();
        let media_binding_id = self.id_generator.generate_media_binding_id();
        let deadline = parse_deadline(request.deadline)?;
        let requirements = build_media_requirements(
            &device,
            &channel,
            MediaPurpose::Playback,
            media_session_id,
            std::collections::BTreeMap::new(),
        );

        let reservation = self
            .media_port
            .reserve_playback(
                tenant_id,
                device_id,
                channel_id,
                media_session_id,
                media_binding_id,
                start_time,
                end_time,
                request.scale,
                &requirements,
                self.clock.as_ref(),
            )
            .await?;

        let result = async {
            let (operation, op_event) = Operation::new(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                scope.idempotency_key.clone(),
                device_id,
                scope.target,
                CommandPayload::StartPlayback {
                    media_session_id,
                    channel_id,
                    media_node_id: reservation.media_node_id,
                    start_time,
                    end_time,
                    scale: request.scale,
                },
                deadline,
                owner.owner_epoch,
            )
            .map_err(crate::SignalError::from)?;

            let (mut session, session_event) = MediaSession::new(
                self.clock.as_ref(),
                media_session_id,
                tenant_id,
                device_id,
                channel_id,
                MediaPurpose::Playback,
                MediaSessionDesiredState::Active,
                owner.owner_epoch,
                operation.operation_id(),
                operation.idempotency_scope().clone(),
                deadline,
            )
            .map_err(crate::SignalError::from)?;
            session.set_playback_window(start_time, end_time, request.scale);

            let (binding, binding_event) = MediaBinding::new(
                self.clock.as_ref(),
                media_binding_id,
                media_session_id,
                tenant_id,
                channel_id,
                reservation.media_node_id,
                owner.owner_epoch,
                reservation.media_node_instance_epoch,
            )
            .map_err(crate::SignalError::from)?;

            uow.operation_repository().save(&operation).await?;
            uow.media_session_repository().save(&session).await?;
            uow.media_binding_repository().save(&binding).await?;

            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    operation_resource_ref(tenant_id, operation.operation_id()),
                    operation.revision().0,
                    op_event,
                ))
                .await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_session_resource_ref(tenant_id, session.media_session_id()),
                    session.revision().0,
                    session_event,
                ))
                .await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_binding_resource_ref(tenant_id, binding.media_binding_id()),
                    binding.revision().0,
                    binding_event,
                ))
                .await?;

            let allocating_event = session.allocating(self.clock.as_ref())?;
            uow.media_session_repository().save(&session).await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_session_resource_ref(tenant_id, session.media_session_id()),
                    session.revision().0,
                    allocating_event,
                ))
                .await?;

            uow.commit().await?;
            Ok((
                operation.operation_id(),
                session.media_session_id(),
                binding.media_binding_id(),
                reservation,
                owner.owner_epoch,
                deadline,
                scope.idempotency_key.clone(),
                operation.command().payload().clone(),
            ))
        }
        .await;

        let released = if result.is_err() {
            self.media_port
                .release(tenant_id, media_binding_id, self.clock.as_ref())
                .await
        } else {
            Ok(())
        };
        if let Err(e) = released {
            tracing::warn!("failed to release media reservation after failed start_playback: {e}");
        }

        match result {
            Ok((
                operation_id,
                media_session_id,
                media_binding_id,
                reservation,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )) => {
                self.dispatch_media_command(
                    context,
                    uow,
                    operation_id,
                    media_session_id,
                    media_binding_id,
                    &reservation,
                    owner_epoch,
                    deadline,
                    idempotency_key,
                    payload,
                )
                .await
            }
            Err(e) => Err(e),
        }
    }

    /// Starts a two-way talk session.
    #[allow(clippy::large_futures)]
    pub async fn start_talk(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        request: StartTalkRequest,
    ) -> crate::Result<MediaSessionDto> {
        let tenant_id = context.tenant_id;
        let device_id = request.device_id.parse::<DeviceId>()?;
        let channel_id = request.channel_id.parse::<ChannelId>()?;

        let (device, channel) = self
            .ensure_device_and_channel_ready(uow, tenant_id, device_id, channel_id)
            .await?;

        let target = channel_resource_ref(tenant_id, channel_id);
        let scope = IdempotencyScope::new(
            tenant_id,
            context.principal.id.clone(),
            target,
            request.idempotency_key,
        )
        .map_err(crate::SignalError::from)?;

        if let Some(existing) = uow
            .media_session_repository()
            .get_by_idempotency(&scope)
            .await?
        {
            return Ok(MediaSessionDto::from(&existing));
        }

        let owner = self
            .owner_resolver
            .resolve(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::no_owner(device_id.to_string()))
            })?;

        let media_session_id = self.id_generator.generate_media_session_id();
        let media_binding_id = self.id_generator.generate_media_binding_id();
        let deadline = parse_deadline(request.deadline)?;
        let requirements = build_media_requirements(
            &device,
            &channel,
            MediaPurpose::Talk,
            media_session_id,
            std::collections::BTreeMap::new(),
        );

        let reservation = self
            .media_port
            .reserve_talk(
                tenant_id,
                device_id,
                channel_id,
                media_session_id,
                media_binding_id,
                &requirements,
                self.clock.as_ref(),
            )
            .await?;

        let result = async {
            let (operation, op_event) = Operation::new(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                scope.idempotency_key.clone(),
                device_id,
                scope.target,
                CommandPayload::StartTalk {
                    media_session_id,
                    channel_id,
                    media_node_id: reservation.media_node_id,
                },
                deadline,
                owner.owner_epoch,
            )
            .map_err(crate::SignalError::from)?;

            let (mut session, session_event) = MediaSession::new(
                self.clock.as_ref(),
                media_session_id,
                tenant_id,
                device_id,
                channel_id,
                MediaPurpose::Talk,
                MediaSessionDesiredState::Active,
                owner.owner_epoch,
                operation.operation_id(),
                operation.idempotency_scope().clone(),
                deadline,
            )
            .map_err(crate::SignalError::from)?;

            let (binding, binding_event) = MediaBinding::new(
                self.clock.as_ref(),
                media_binding_id,
                media_session_id,
                tenant_id,
                channel_id,
                reservation.media_node_id,
                owner.owner_epoch,
                reservation.media_node_instance_epoch,
            )
            .map_err(crate::SignalError::from)?;

            uow.operation_repository().save(&operation).await?;
            uow.media_session_repository().save(&session).await?;
            uow.media_binding_repository().save(&binding).await?;

            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    operation_resource_ref(tenant_id, operation.operation_id()),
                    operation.revision().0,
                    op_event,
                ))
                .await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_session_resource_ref(tenant_id, session.media_session_id()),
                    session.revision().0,
                    session_event,
                ))
                .await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_binding_resource_ref(tenant_id, binding.media_binding_id()),
                    binding.revision().0,
                    binding_event,
                ))
                .await?;

            let allocating_event = session.allocating(self.clock.as_ref())?;
            uow.media_session_repository().save(&session).await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    media_session_resource_ref(tenant_id, session.media_session_id()),
                    session.revision().0,
                    allocating_event,
                ))
                .await?;

            uow.commit().await?;
            Ok((
                operation.operation_id(),
                session.media_session_id(),
                binding.media_binding_id(),
                reservation,
                owner.owner_epoch,
                deadline,
                scope.idempotency_key.clone(),
                operation.command().payload().clone(),
            ))
        }
        .await;

        let released = if result.is_err() {
            self.media_port
                .release(tenant_id, media_binding_id, self.clock.as_ref())
                .await
        } else {
            Ok(())
        };
        if let Err(e) = released {
            tracing::warn!("failed to release media reservation after failed start_talk: {e}");
        }

        match result {
            Ok((
                operation_id,
                media_session_id,
                media_binding_id,
                reservation,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )) => {
                self.dispatch_media_command(
                    context,
                    uow,
                    operation_id,
                    media_session_id,
                    media_binding_id,
                    &reservation,
                    owner_epoch,
                    deadline,
                    idempotency_key,
                    payload,
                )
                .await
            }
            Err(e) => Err(e),
        }
    }
}

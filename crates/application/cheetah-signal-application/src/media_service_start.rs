//! Media start operations.

use crate::dto::{
    MediaSessionDto, StartBroadcastRequest, StartLiveRequest, StartPlaybackRequest,
    StartTalkRequest,
};
use crate::media_service::*;
use cheetah_domain::{
    CommandPayload, DomainError, IdempotencyScope, MediaBinding, MediaPurpose, MediaSession,
    MediaSessionDesiredState, Operation, UnitOfWork,
};
use cheetah_signal_types::{
    ChannelId, Deadline, DeviceId, MediaBindingId, MediaSessionId, OwnerEpoch, RequestContext,
    UtcTimestamp,
};

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

        // WF-002 step 1: validate device/channel readiness and idempotency.
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

        // Persist nothing yet; close the read transaction before external calls.
        uow.commit().await?;

        // WF-002 step 2: resolve owner.
        let owner = self
            .owner_resolver
            .resolve(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::no_owner(device_id.to_string()))
            })?;

        // WF-002 step 3: reserve media node (outside a DB transaction).
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

        // WF-002 step 4-6: atomically create Pending Operation, Requested MediaSession,
        // Reserved MediaBinding and outbox, then transition to Allocating.
        let payload = CommandPayload::StartLive {
            media_session_id,
            channel_id,
            media_node_id: reservation.media_node_id,
            purpose: MediaPurpose::Live,
        };
        let result = self
            .persist_start_resources(
                context,
                uow,
                scope,
                device_id,
                channel_id,
                media_session_id,
                media_binding_id,
                &reservation,
                owner.owner_epoch,
                deadline,
                payload,
                MediaPurpose::Live,
                None,
            )
            .await;

        let released = if result.is_err() {
            self.media_port
                .release(tenant_id, media_binding_id, self.clock.as_ref())
                .await
        } else {
            Ok(())
        };
        if let Err(e) = released {
            tracing::warn!(
                tenant_id = %tenant_id,
                binding_id = %media_binding_id,
                "failed to release media reservation after failed start_live: {e}"
            );
        }

        // WF-002 step 7-11: dispatch command and apply StreamOnline/Completed/Failed.
        match result {
            Ok((
                operation,
                _session,
                _binding,
                reservation,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )) => {
                self.dispatch_media_command(
                    context,
                    uow,
                    operation.operation_id(),
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

    /// Atomically persists the Pending Operation, Requested MediaSession and
    /// Reserved MediaBinding for a start request, then transitions the session to
    /// Allocating. All writes and outbox events are committed in one transaction.
    #[allow(clippy::too_many_arguments)]
    async fn persist_start_resources(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        scope: IdempotencyScope,
        device_id: DeviceId,
        channel_id: ChannelId,
        media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        reservation: &cheetah_domain::MediaReservation,
        owner_epoch: OwnerEpoch,
        deadline: Option<Deadline>,
        payload: CommandPayload,
        purpose: MediaPurpose,
        playback: Option<(UtcTimestamp, UtcTimestamp, f64)>,
    ) -> crate::Result<(
        Operation,
        MediaSession,
        MediaBinding,
        cheetah_domain::MediaReservation,
        OwnerEpoch,
        Option<Deadline>,
        String,
        CommandPayload,
    )> {
        let tenant_id = context.tenant_id;

        let (operation, op_event) = Operation::new(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            scope.idempotency_key.clone(),
            device_id,
            scope.target,
            payload.clone(),
            deadline,
            owner_epoch,
        )
        .map_err(crate::SignalError::from)?;

        let (mut session, session_event) = MediaSession::new(
            self.clock.as_ref(),
            media_session_id,
            tenant_id,
            device_id,
            channel_id,
            purpose,
            MediaSessionDesiredState::Active,
            owner_epoch,
            operation.operation_id(),
            operation.idempotency_scope().clone(),
            deadline,
        )
        .map_err(crate::SignalError::from)?;

        if let Some((start_time, end_time, scale)) = playback {
            session.set_playback_window(start_time, end_time, scale);
        }

        let (binding, binding_event) = MediaBinding::new(
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

        let idempotency_key = operation.idempotency_scope().idempotency_key.clone();
        Ok((
            operation,
            session,
            binding,
            reservation.clone(),
            owner_epoch,
            deadline,
            idempotency_key,
            payload,
        ))
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

        // Persist nothing yet; close the read transaction before external calls.
        uow.commit().await?;

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

        let payload = CommandPayload::StartPlayback {
            media_session_id,
            channel_id,
            media_node_id: reservation.media_node_id,
            start_time,
            end_time,
            scale: request.scale,
        };
        let result = self
            .persist_start_resources(
                context,
                uow,
                scope,
                device_id,
                channel_id,
                media_session_id,
                media_binding_id,
                &reservation,
                owner.owner_epoch,
                deadline,
                payload,
                MediaPurpose::Playback,
                Some((start_time, end_time, request.scale)),
            )
            .await;

        let released = if result.is_err() {
            self.media_port
                .release(tenant_id, media_binding_id, self.clock.as_ref())
                .await
        } else {
            Ok(())
        };
        if let Err(e) = released {
            tracing::warn!(
                tenant_id = %tenant_id,
                binding_id = %media_binding_id,
                "failed to release media reservation after failed start_playback: {e}"
            );
        }

        match result {
            Ok((
                operation,
                _session,
                _binding,
                reservation,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )) => {
                self.dispatch_media_command(
                    context,
                    uow,
                    operation.operation_id(),
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

        // Persist nothing yet; close the read transaction before external calls.
        uow.commit().await?;

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

        let payload = CommandPayload::StartTalk {
            media_session_id,
            channel_id,
            media_node_id: reservation.media_node_id,
        };
        let result = self
            .persist_start_resources(
                context,
                uow,
                scope,
                device_id,
                channel_id,
                media_session_id,
                media_binding_id,
                &reservation,
                owner.owner_epoch,
                deadline,
                payload,
                MediaPurpose::Talk,
                None,
            )
            .await;

        let released = if result.is_err() {
            self.media_port
                .release(tenant_id, media_binding_id, self.clock.as_ref())
                .await
        } else {
            Ok(())
        };
        if let Err(e) = released {
            tracing::warn!(
                tenant_id = %tenant_id,
                binding_id = %media_binding_id,
                "failed to release media reservation after failed start_talk: {e}"
            );
        }

        match result {
            Ok((
                operation,
                _session,
                _binding,
                reservation,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )) => {
                self.dispatch_media_command(
                    context,
                    uow,
                    operation.operation_id(),
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

    /// Starts a one-way voice broadcast to a device.
    ///
    /// Broadcast reuses the talk media-sender resource but negotiates a
    /// `sendonly` audio dialog (platform to device). The saga is identical to
    /// [`MediaService::start_talk`]: validate readiness and idempotency, reserve
    /// the media sender outside a transaction, atomically persist the operation,
    /// session and binding, and compensate the reservation on failure.
    #[allow(clippy::large_futures)]
    pub async fn start_broadcast(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        request: StartBroadcastRequest,
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

        // Persist nothing yet; close the read transaction before external calls.
        uow.commit().await?;

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
            MediaPurpose::Broadcast,
            media_session_id,
            std::collections::BTreeMap::new(),
        );

        let reservation = self
            .media_port
            .reserve_broadcast(
                tenant_id,
                device_id,
                channel_id,
                media_session_id,
                media_binding_id,
                &requirements,
                self.clock.as_ref(),
            )
            .await?;

        let payload = CommandPayload::StartBroadcast {
            media_session_id,
            channel_id,
            media_node_id: reservation.media_node_id,
        };
        let result = self
            .persist_start_resources(
                context,
                uow,
                scope,
                device_id,
                channel_id,
                media_session_id,
                media_binding_id,
                &reservation,
                owner.owner_epoch,
                deadline,
                payload,
                MediaPurpose::Broadcast,
                None,
            )
            .await;

        let released = if result.is_err() {
            self.media_port
                .release(tenant_id, media_binding_id, self.clock.as_ref())
                .await
        } else {
            Ok(())
        };
        if let Err(e) = released {
            tracing::warn!(
                tenant_id = %tenant_id,
                binding_id = %media_binding_id,
                "failed to release media reservation after failed start_broadcast: {e}"
            );
        }

        match result {
            Ok((
                operation,
                _session,
                _binding,
                reservation,
                owner_epoch,
                deadline,
                idempotency_key,
                payload,
            )) => {
                self.dispatch_media_command(
                    context,
                    uow,
                    operation.operation_id(),
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

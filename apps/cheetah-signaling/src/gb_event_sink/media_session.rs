//! Media-session lifecycle transitions driven by GB28181 driver events.

use cheetah_domain::{DomainEvent, MediaSession, MediaSessionError, MediaSessionState, UnitOfWork};
use cheetah_http_api::state::ApiState;
use cheetah_signal_types::{
    DeviceId, Event, MediaSessionId, RequestContext, ResourceId, ResourceKind, ResourceRef,
    SignalError, TenantId,
};
use std::collections::BTreeMap;

use super::outbox::build_gb_event;
use super::storage_error;

/// Desired media session transition requested by a GB28181 driver event.
pub(super) enum MediaSessionTransition {
    /// Progress the session to Active.
    Start,
    /// Tear the session down.
    Stop,
    /// Fail the session with the given reason.
    Fail(String),
}

/// Persists a single [`MediaSession`] state transition and appends the resulting
/// `MediaSessionStateChanged` event to the outbox in the same UnitOfWork.
async fn save_and_append_media_session_transition(
    uow: &mut dyn UnitOfWork,
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    media_session_id: MediaSessionId,
    session: &MediaSession,
    event: DomainEvent,
) -> Result<(), SignalError> {
    uow.media_session_repository().save(session).await?;
    let aggregate_ref = ResourceRef {
        tenant_id,
        kind: ResourceKind::MediaSession,
        id: ResourceId::MediaSession(media_session_id),
    };
    uow.outbox()
        .append(Event::new(
            state.id_generator.as_ref(),
            state.clock.as_ref(),
            context,
            tenant_id,
            aggregate_ref,
            session.revision().0,
            event,
        ))
        .await?;
    Ok(())
}

/// Drives a [`MediaSession`] through the requested transition, saving and
/// appending each resulting `MediaSessionStateChanged` event one step at a time
/// so the repository's optimistic-concurrency check succeeds. The
/// `Gb28181EventReceived` envelope is always appended in the same UnitOfWork.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_media_session_event(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    media_session_id: MediaSessionId,
    transition: MediaSessionTransition,
    device_id: Option<DeviceId>,
    external_id: Option<&str>,
    event_type: &str,
    payload: BTreeMap<String, String>,
) -> Result<(), SignalError> {
    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let session = uow
        .media_session_repository()
        .get(tenant_id, media_session_id)
        .await?;

    if let Some(mut session) = session {
        let clock = state.clock.as_ref();

        match transition {
            MediaSessionTransition::Start => match session.state() {
                MediaSessionState::Requested => {
                    let event = session.allocating(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                    let event = session.inviting(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                    let event = session.active(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                MediaSessionState::Allocating => {
                    let event = session.inviting(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                    let event = session.active(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                MediaSessionState::Inviting => {
                    let event = session.active(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                _ => {}
            },
            MediaSessionTransition::Stop => match session.state() {
                MediaSessionState::Active => {
                    let event = session.stop(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                    if session.state() == MediaSessionState::Stopping {
                        let event = session.stopped(clock)?;
                        save_and_append_media_session_transition(
                            &mut *uow,
                            state,
                            context,
                            tenant_id,
                            media_session_id,
                            &session,
                            event,
                        )
                        .await?;
                    }
                }
                MediaSessionState::Stopping => {
                    let event = session.stopped(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                MediaSessionState::Requested
                | MediaSessionState::Allocating
                | MediaSessionState::Inviting => {
                    let event = session.stop(clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
                _ => {}
            },
            MediaSessionTransition::Fail(reason) => {
                if !session.state().is_terminal() {
                    let event = session.failed(MediaSessionError::new("gb28181", reason), clock)?;
                    save_and_append_media_session_transition(
                        &mut *uow,
                        state,
                        context,
                        tenant_id,
                        media_session_id,
                        &session,
                        event,
                    )
                    .await?;
                }
            }
        };
    }

    // Always append the GB28181 envelope so the driver event is recorded even
    // when the session is unknown or already in a terminal state.
    let envelope = build_gb_event(
        state,
        context,
        tenant_id,
        device_id,
        external_id,
        event_type,
        payload,
    );
    uow.outbox().append(envelope).await?;
    uow.commit().await?;
    Ok(())
}

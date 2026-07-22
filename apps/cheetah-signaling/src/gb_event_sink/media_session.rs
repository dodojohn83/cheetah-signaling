//! Media-session lifecycle transitions driven by GB28181 driver events.

use cheetah_domain::{DomainEvent, MediaSessionError, MediaSessionState};
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

/// Drives a [`cheetah_domain::MediaSession`] through the requested transition,
/// appends each resulting `MediaSessionStateChanged` event with the revision
/// captured at the moment the transition occurred, and appends a
/// `Gb28181EventReceived` envelope in the same UnitOfWork.
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
    let mut state_events: Vec<Event<DomainEvent>> = Vec::new();

    if let Some(mut session) = session {
        let clock = state.clock.as_ref();
        let aggregate_ref = ResourceRef {
            tenant_id,
            kind: ResourceKind::MediaSession,
            id: ResourceId::MediaSession(media_session_id),
        };

        let push_transition =
            |events: &mut Vec<Event<DomainEvent>>, event: DomainEvent, revision: u64| {
                events.push(Event::new(
                    state.id_generator.as_ref(),
                    clock,
                    context,
                    tenant_id,
                    aggregate_ref.clone(),
                    revision,
                    event,
                ));
            };

        match transition {
            MediaSessionTransition::Start => match session.state() {
                MediaSessionState::Requested => {
                    let event = session.allocating(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                    let event = session.inviting(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                    let event = session.active(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                }
                MediaSessionState::Allocating => {
                    let event = session.inviting(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                    let event = session.active(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                }
                MediaSessionState::Inviting => {
                    let event = session.active(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                }
                _ => {}
            },
            MediaSessionTransition::Stop => match session.state() {
                MediaSessionState::Active => {
                    let event = session.stop(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                    if session.state() == MediaSessionState::Stopping {
                        let event = session.stopped(clock)?;
                        push_transition(&mut state_events, event, session.revision().0);
                    }
                }
                MediaSessionState::Stopping => {
                    let event = session.stopped(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                }
                MediaSessionState::Requested
                | MediaSessionState::Allocating
                | MediaSessionState::Inviting => {
                    let event = session.stop(clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                }
                _ => {}
            },
            MediaSessionTransition::Fail(reason) => {
                if !session.state().is_terminal() {
                    let event = session.failed(MediaSessionError::new("gb28181", reason), clock)?;
                    push_transition(&mut state_events, event, session.revision().0);
                }
            }
        };

        if !state_events.is_empty() {
            uow.media_session_repository().save(&session).await?;
            for event in state_events {
                uow.outbox().append(event).await?;
            }
        }
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

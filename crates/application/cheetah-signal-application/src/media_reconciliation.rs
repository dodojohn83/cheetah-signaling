//! Shared helpers for media session and binding reconciliation.

use crate::media_service::{media_binding_resource_ref, media_session_resource_ref, wrap_event};
use cheetah_domain::{
    DomainEvent, MediaBinding, MediaBindingError, MediaBindingState, MediaSession,
    MediaSessionError, MediaSessionState, UnitOfWork,
};
use cheetah_signal_types::{Clock, IdGenerator, RequestContext};

/// Appends a media session domain event to the outbox.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn append_session_event(
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &MediaSession,
    event: DomainEvent,
) -> crate::Result<()> {
    uow.outbox()
        .append(wrap_event(
            id_generator,
            clock,
            context,
            context.tenant_id,
            media_session_resource_ref(context.tenant_id, session.media_session_id()),
            session.revision().0,
            event,
        ))
        .await
        .map_err(crate::SignalError::from)
}

/// Appends a media binding domain event to the outbox.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn append_binding_event(
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    binding: &MediaBinding,
    event: DomainEvent,
) -> crate::Result<()> {
    uow.outbox()
        .append(wrap_event(
            id_generator,
            clock,
            context,
            context.tenant_id,
            media_binding_resource_ref(context.tenant_id, binding.media_binding_id()),
            binding.revision().0,
            event,
        ))
        .await
        .map_err(crate::SignalError::from)
}

/// Releases a stopped or terminal session and its binding.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn release_session_and_binding(
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &mut MediaSession,
    binding: &mut MediaBinding,
) -> crate::Result<()> {
    if !session.is_terminal() {
        let ev = session.stop(clock)?;
        append_session_event(id_generator, clock, context, uow, session, ev).await?;
        if session.state() == MediaSessionState::Stopping {
            let ev = session.stopped(clock)?;
            append_session_event(id_generator, clock, context, uow, session, ev).await?;
        }
    }

    if !binding.is_terminal() {
        if binding.state() != MediaBindingState::Releasing {
            let ev = binding.release(clock)?;
            append_binding_event(id_generator, clock, context, uow, binding, ev).await?;
        }
        let ev = binding.released(clock)?;
        append_binding_event(id_generator, clock, context, uow, binding, ev).await?;
    }

    Ok(())
}

/// Fails a session and its binding with a stable error code.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn fail_session_and_binding(
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &mut MediaSession,
    binding: &mut MediaBinding,
    code: &str,
    message: &str,
) -> crate::Result<()> {
    if !session.is_terminal() {
        let ev = session.failed(MediaSessionError::new(code, message), clock)?;
        append_session_event(id_generator, clock, context, uow, session, ev).await?;
    }

    if !binding.is_terminal() {
        let ev = binding.failed(MediaBindingError::new(code, message), clock)?;
        append_binding_event(id_generator, clock, context, uow, binding, ev).await?;
    }

    Ok(())
}

/// Converges an active session and its binding to the `Active` state.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn converge_active(
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &mut MediaSession,
    binding: &mut MediaBinding,
) -> crate::Result<()> {
    match session.state() {
        MediaSessionState::Requested => {
            let ev = session.allocating(clock)?;
            append_session_event(id_generator, clock, context, uow, session, ev).await?;
            let ev = session.inviting(clock)?;
            append_session_event(id_generator, clock, context, uow, session, ev).await?;
            let ev = session.active(clock)?;
            append_session_event(id_generator, clock, context, uow, session, ev).await?;
        }
        MediaSessionState::Allocating => {
            let ev = session.inviting(clock)?;
            append_session_event(id_generator, clock, context, uow, session, ev).await?;
            let ev = session.active(clock)?;
            append_session_event(id_generator, clock, context, uow, session, ev).await?;
        }
        MediaSessionState::Inviting => {
            let ev = session.active(clock)?;
            append_session_event(id_generator, clock, context, uow, session, ev).await?;
        }
        MediaSessionState::Active => {}
        _ => {
            return Err(crate::SignalError::from(
                cheetah_domain::DomainError::invalid_transition(
                    "MediaSession",
                    format!("{:?}", session.state()),
                    "Active",
                ),
            ));
        }
    }

    if binding.state() == MediaBindingState::Reserved {
        let ev = binding.activate(clock)?;
        append_binding_event(id_generator, clock, context, uow, binding, ev).await?;
    }

    Ok(())
}

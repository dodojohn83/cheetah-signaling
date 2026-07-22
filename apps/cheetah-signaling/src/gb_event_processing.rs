//! GB28181 event processing.
//!
//! Maps inbound `Gb28181Event`s to application service calls and outbox events.

use crate::gb_catalog_buffer::{CatalogBuffer, RecordInfoBuffer};
use crate::gb_event_context::{build_context, control_outcome, serialize_record_items};
use crate::gb_event_sink::{
    append_gb_event, build_gb_event, ensure_online, mark_offline, replace_catalog,
    resolve_device_id, storage_error, submit_bootstrap_queries, update_device_info,
};
use cheetah_domain::{DomainEvent, MediaSession, MediaSessionError, MediaSessionState, UnitOfWork};
use cheetah_gb28181_module::Gb28181Event;
use cheetah_http_api::state::ApiState;
use cheetah_signal_types::{
    DeviceId, Event, GbCommandMethod, GbMetricsRecorder, MediaSessionId, NodeId, RequestContext,
    ResourceId, ResourceKind, ResourceRef, SignalError, TenantId,
};
use std::collections::BTreeMap;
use tracing::warn;

/// Desired media session transition requested by a GB28181 driver event.
pub(crate) enum MediaSessionTransition {
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
pub(crate) async fn handle_media_session_event(
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

pub(crate) async fn process_event(
    state: &ApiState,
    node_id: NodeId,
    tenant_id: Option<TenantId>,
    event: Gb28181Event,
    catalog_buffer: &mut CatalogBuffer,
    record_buffer: &mut RecordInfoBuffer,
    gb_metrics: &dyn GbMetricsRecorder,
) -> Result<(), SignalError> {
    let tenant_id = match tenant_id {
        Some(id) => id,
        None => {
            warn!("dropping gb28181 event; no default_tenant_id configured");
            return Ok(());
        }
    };

    let context = build_context(state, node_id, tenant_id, &event);

    match event {
        Gb28181Event::DeviceRegistered {
            device_id,
            registration_sequence,
            ..
        } => match ensure_online(state, &context, tenant_id, &device_id, true).await {
            Ok(Some(internal_id)) => {
                submit_bootstrap_queries(
                    state,
                    &context,
                    tenant_id,
                    internal_id,
                    registration_sequence,
                )
                .await
            }
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        },
        Gb28181Event::DeviceUnregistered { device_id, .. } => {
            mark_offline(state, &context, tenant_id, &device_id).await
        }
        Gb28181Event::DevicePresenceChanged {
            device_id,
            presence,
            ..
        } => match presence {
            cheetah_gb28181_module::DevicePresence::Online => {
                ensure_online(state, &context, tenant_id, &device_id, true)
                    .await
                    .map(|_| ())
            }
            cheetah_gb28181_module::DevicePresence::Offline => {
                mark_offline(state, &context, tenant_id, &device_id).await
            }
        },
        Gb28181Event::Keepalive { device_id, .. } => {
            ensure_online(state, &context, tenant_id, &device_id, false)
                .await
                .map(|_| ())
        }
        Gb28181Event::CatalogReceived {
            device_id,
            sn,
            sum_num,
            num,
            items,
            ..
        } => {
            gb_metrics.record_catalog_fragment();
            match catalog_buffer.accumulate(tenant_id, &device_id, &sn, sum_num, num, items) {
                Some(merged) => {
                    replace_catalog(state, &context, tenant_id, &device_id, &merged).await
                }
                None => Ok(()),
            }
        }
        Gb28181Event::DeviceInfoReceived {
            device_id,
            result,
            manufacturer,
            model,
            firmware,
            ..
        } => {
            let mut metadata = BTreeMap::new();
            if let Some(v) = result {
                metadata.insert("info_result".to_string(), v.clone());
            }
            if let Some(v) = manufacturer {
                metadata.insert("manufacturer".to_string(), v.clone());
            }
            if let Some(v) = model {
                metadata.insert("model".to_string(), v.clone());
            }
            if let Some(v) = firmware {
                metadata.insert("firmware".to_string(), v.clone());
            }
            update_device_info(state, &context, tenant_id, &device_id, metadata).await
        }
        Gb28181Event::DeviceStatusReceived {
            device_id,
            result,
            online,
            status,
            reason,
            invalid_equip,
            ..
        } => {
            let mut metadata = BTreeMap::new();
            if let Some(v) = result {
                metadata.insert("status_result".to_string(), v.clone());
            }
            if let Some(v) = online {
                metadata.insert("online".to_string(), v.clone());
            }
            if let Some(v) = status {
                metadata.insert("status".to_string(), v.clone());
            }
            if let Some(v) = reason {
                metadata.insert("reason".to_string(), v.clone());
            }
            if let Some(v) = invalid_equip {
                metadata.insert("invalid_equip".to_string(), v.clone());
            }
            update_device_info(state, &context, tenant_id, &device_id, metadata).await
        }
        Gb28181Event::AlarmReceived {
            device_id,
            sn,
            priority,
            method,
            alarm_type,
            time,
            info,
            ..
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = priority {
                payload.insert("priority".to_string(), v);
            }
            if let Some(v) = method {
                payload.insert("method".to_string(), v);
            }
            if let Some(v) = alarm_type {
                payload.insert("alarm_type".to_string(), v);
            }
            if let Some(v) = time {
                payload.insert("time".to_string(), v);
            }
            if let Some(v) = info {
                payload.insert("info".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "Alarm",
                payload,
            )
            .await
        }
        Gb28181Event::MobilePositionReceived {
            device_id,
            sn,
            time,
            longitude,
            latitude,
            speed,
            direction,
            altitude,
            ..
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = time {
                payload.insert("time".to_string(), v);
            }
            if let Some(v) = longitude {
                payload.insert("longitude".to_string(), v);
            }
            if let Some(v) = latitude {
                payload.insert("latitude".to_string(), v);
            }
            if let Some(v) = speed {
                payload.insert("speed".to_string(), v);
            }
            if let Some(v) = direction {
                payload.insert("direction".to_string(), v);
            }
            if let Some(v) = altitude {
                payload.insert("altitude".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "MobilePosition",
                payload,
            )
            .await
        }
        Gb28181Event::DeviceControlResponseReceived {
            device_id,
            sn,
            result,
            ..
        } => {
            gb_metrics.record_command(GbCommandMethod::DeviceControl, control_outcome(&result));
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("sn".to_string(), sn);
            if let Some(v) = result {
                payload.insert("result".to_string(), v);
            }
            append_gb_event(
                state,
                &context,
                tenant_id,
                internal_id,
                Some(device_id.as_ref()),
                "DeviceControl",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionStarted {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
            remote_sdp,
            remote_ssrc,
            remote_port,
            remote_proto,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            payload.insert("remote_address".to_string(), source.to_string());
            payload.insert("remote_sdp".to_string(), remote_sdp);
            if let Some(v) = remote_ssrc {
                payload.insert("remote_ssrc".to_string(), v);
            }
            payload.insert("remote_port".to_string(), remote_port.to_string());
            payload.insert("remote_proto".to_string(), remote_proto);
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Start,
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionStarted",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionStopped {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            if let Some(s) = source {
                payload.insert("remote_address".to_string(), s.to_string());
            }
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Stop,
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionStopped",
                payload,
            )
            .await
        }
        Gb28181Event::MediaSessionFailed {
            media_session_id,
            domain_id,
            device_id,
            channel_id,
            source,
            reason,
        } => {
            let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("media_session_id".to_string(), media_session_id.to_string());
            payload.insert("channel_id".to_string(), channel_id.to_string());
            payload.insert("device_id".to_string(), device_id.to_string());
            if let Some(s) = source {
                payload.insert("remote_address".to_string(), s.to_string());
            }
            payload.insert("reason".to_string(), reason.clone());
            handle_media_session_event(
                state,
                &context,
                tenant_id,
                media_session_id,
                MediaSessionTransition::Fail(reason),
                internal_id,
                Some(device_id.as_ref()),
                "MediaSessionFailed",
                payload,
            )
            .await
        }
        Gb28181Event::RecordInfoReceived {
            device_id,
            sn,
            num,
            sum_num,
            items,
            ..
        } => {
            if let Some(records) =
                record_buffer.accumulate(tenant_id, &device_id, &sn, sum_num, num, items)
            {
                let internal_id = resolve_device_id(state, tenant_id, device_id.as_ref()).await;
                let mut payload = BTreeMap::new();
                payload.insert("sn".to_string(), sn);
                payload.insert("sum_num".to_string(), sum_num.to_string());
                payload.insert("num".to_string(), num.to_string());
                payload.insert("record_count".to_string(), records.len().to_string());
                if !records.is_empty()
                    && let Some(json) = serialize_record_items(&records)
                {
                    payload.insert("records".to_string(), json);
                }
                append_gb_event(
                    state,
                    &context,
                    tenant_id,
                    internal_id,
                    Some(device_id.as_ref()),
                    "RecordInfo",
                    payload,
                )
                .await?;
            }
            Ok(())
        }
        Gb28181Event::CascadePlatformConnected {
            domain_id,
            platform_id,
            upstream,
            expires,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("upstream".to_string(), upstream);
            payload.insert("expires".to_string(), expires.to_string());
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlatformConnected",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlatformDisconnected {
            domain_id,
            platform_id,
            reason,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("reason".to_string(), reason);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlatformDisconnected",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlayRequested {
            domain_id,
            platform_id,
            bridge_id,
            upstream_call_id,
            upstream_from,
            upstream_to,
            target_user,
            remote_sdp,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("bridge_id".to_string(), bridge_id.clone());
            payload.insert("upstream_call_id".to_string(), upstream_call_id);
            payload.insert("upstream_from".to_string(), upstream_from);
            payload.insert("upstream_to".to_string(), upstream_to);
            payload.insert("target_user".to_string(), target_user);
            payload.insert("remote_sdp".to_string(), remote_sdp);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlayRequested",
                payload,
            )
            .await
        }
        Gb28181Event::CascadePlayStopped {
            domain_id,
            platform_id,
            bridge_id,
            reason,
        } => {
            let mut payload = BTreeMap::new();
            payload.insert("domain_id".to_string(), domain_id.to_string());
            payload.insert("platform_id".to_string(), platform_id.clone());
            payload.insert("bridge_id".to_string(), bridge_id.clone());
            payload.insert("reason".to_string(), reason);
            append_gb_event(
                state,
                &context,
                tenant_id,
                None,
                Some(&platform_id),
                "CascadePlayStopped",
                payload,
            )
            .await
        }
    }
}

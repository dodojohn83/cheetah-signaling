//! Media session HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, IdempotencyKey, JsonBody, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use cheetah_domain::DomainError;
use cheetah_signal_application::dto::{
    ControlPlaybackRequest, MediaSessionDto, StartBroadcastRequest, StartLiveRequest,
    StartPlaybackRequest, StartTalkRequest, StopLiveRequest,
};
use cheetah_signal_types::{
    AuditOutcome, DeviceId, MediaSessionId, Page, SignalError, SignalErrorKind, UtcTimestamp,
};
use std::sync::Arc;

/// Maximum byte length of the `purpose` discriminator in a media session creation request.
const MAX_MEDIA_PURPOSE_BYTES: usize = 64;

pub async fn list_sessions(
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<MediaSessionDto>>, HttpError> {
    ctx.require_scope("viewer")?;
    let page = query.page_request()?;
    let device_id = query
        .device_id
        .as_deref()
        .map(str::parse::<DeviceId>)
        .transpose()
        .map_err(HttpError::from)?;
    let updated_after = query
        .updated_after
        .as_deref()
        .map(UtcTimestamp::parse_rfc3339)
        .transpose()
        .map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = uow
        .media_session_repository()
        .list(
            ctx.tenant_id,
            device_id,
            query.purpose,
            query.status,
            updated_after,
            page,
        )
        .await
        .map_err(HttpError::from)?;
    Ok(Json(result.map(|s| MediaSessionDto::from(&s))))
}

pub async fn create_session(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    JsonBody(body): JsonBody<serde_json::Value>,
) -> Result<axum::response::Response, HttpError> {
    ctx.require_scope("operator")?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let purpose = match body.get("purpose") {
        Some(v) => v.as_str().ok_or_else(|| {
            HttpError::Signal(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "purpose must be a string",
            ))
        })?,
        None => "live",
    };
    if purpose.len() > MAX_MEDIA_PURPOSE_BYTES {
        return Err(HttpError::Signal(SignalError::new(
            SignalErrorKind::InvalidArgument,
            "purpose exceeds maximum length",
        )));
    }
    let purpose_kind = if purpose.eq_ignore_ascii_case("live") {
        "live"
    } else if purpose.eq_ignore_ascii_case("playback") {
        "playback"
    } else if purpose.eq_ignore_ascii_case("talk") {
        "talk"
    } else if purpose.eq_ignore_ascii_case("broadcast") {
        "broadcast"
    } else {
        return Err(HttpError::Signal(SignalError::new(
            SignalErrorKind::InvalidArgument,
            "unsupported media purpose",
        )));
    };

    let result = match purpose_kind {
        "live" => {
            let mut request: StartLiveRequest =
                serde_json::from_value(body).map_err(HttpError::from)?;
            request.idempotency_key = idempotency.0.clone();
            state
                .media_service
                .start_live(&ctx.0, &mut *uow, request)
                .await
                .map_err(HttpError::from)?
        }
        "playback" => {
            let mut request: StartPlaybackRequest =
                serde_json::from_value(body).map_err(HttpError::from)?;
            request.idempotency_key = idempotency.0.clone();
            state
                .media_service
                .start_playback(&ctx.0, &mut *uow, request)
                .await
                .map_err(HttpError::from)?
        }
        "talk" => {
            let mut request: StartTalkRequest =
                serde_json::from_value(body).map_err(HttpError::from)?;
            request.idempotency_key = idempotency.0.clone();
            state
                .media_service
                .start_talk(&ctx.0, &mut *uow, request)
                .await
                .map_err(HttpError::from)?
        }
        "broadcast" => {
            let mut request: StartBroadcastRequest =
                serde_json::from_value(body).map_err(HttpError::from)?;
            request.idempotency_key = idempotency.0.clone();
            state
                .media_service
                .start_broadcast(&ctx.0, &mut *uow, request)
                .await
                .map_err(HttpError::from)?
        }
        _ => unreachable!(),
    };
    let session_id = result.media_session_id.to_string();
    let body = serde_json::to_value(result).map_err(HttpError::from)?;
    crate::audit::record(
        &state,
        &ctx,
        "media.session.create",
        "media_session",
        Some(session_id.clone()),
        None,
        AuditOutcome::Success,
    );
    let mut response = (StatusCode::ACCEPTED, Json(body)).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("/api/v1/media/sessions/{session_id}")) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    Ok(response)
}

pub async fn get_session(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<MediaSessionDto>, HttpError> {
    ctx.require_scope("viewer")?;
    let media_session_id = id.parse::<MediaSessionId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let session = uow
        .media_session_repository()
        .get(ctx.tenant_id, media_session_id)
        .await
        .map_err(HttpError::from)?
        .ok_or_else(|| {
            HttpError::from(DomainError::not_found(
                "media session",
                media_session_id.to_string(),
            ))
        })?;
    Ok(Json(MediaSessionDto::from(&session)))
}

pub async fn stop_session(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    JsonBody(_body): JsonBody<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("operator")?;
    let media_session_id = id.parse::<MediaSessionId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let request = StopLiveRequest {
        media_session_id: media_session_id.to_string(),
        idempotency_key: idempotency.0,
    };
    let result = state
        .media_service
        .stop_live(&ctx.0, &mut *uow, request)
        .await
        .map_err(HttpError::from)?;
    let session_id = result.media_session_id.to_string();
    let body = serde_json::to_value(result).map_err(HttpError::from)?;
    crate::audit::record(
        &state,
        &ctx,
        "media.session.stop",
        "media_session",
        Some(session_id),
        None,
        AuditOutcome::Success,
    );
    Ok(Json(body))
}

pub async fn control_session(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    JsonBody(body): JsonBody<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("operator")?;
    let media_session_id = id.parse::<MediaSessionId>().map_err(HttpError::from)?;
    let mut request: ControlPlaybackRequest =
        serde_json::from_value(body).map_err(HttpError::from)?;
    request.media_session_id = media_session_id.to_string();
    request.idempotency_key = idempotency.0;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = state
        .media_service
        .control_playback(&ctx.0, &mut *uow, request)
        .await
        .map_err(HttpError::from)?;
    let operation_id = result.operation_id.to_string();
    let body = serde_json::to_value(result).map_err(HttpError::from)?;
    crate::audit::record(
        &state,
        &ctx,
        "media.session.control",
        "media_session",
        Some(media_session_id.to_string()),
        Some(operation_id),
        AuditOutcome::Success,
    );
    Ok(Json(body))
}

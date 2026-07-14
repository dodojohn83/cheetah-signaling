//! Media session HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, IdempotencyKey, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use cheetah_signal_application::dto::{
    ControlPlaybackRequest, StartLiveRequest, StartPlaybackRequest, StartTalkRequest,
    StopLiveRequest,
};
use cheetah_signal_types::{DeviceId, MediaSessionId, Page, UtcTimestamp};
use std::sync::Arc;

pub async fn list_sessions(
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<cheetah_signal_application::dto::MediaSessionDto>>, HttpError> {
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
            query.protocol,
            query.status,
            updated_after,
            page,
        )
        .await
        .map_err(HttpError::from)?;
    Ok(Json(result.map(|s| {
        cheetah_signal_application::dto::MediaSessionDto::from(&s)
    })))
}

pub async fn create_session(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), HttpError> {
    ctx.require_scope("operator")?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let purpose = body
        .get("purpose")
        .and_then(|v| v.as_str())
        .unwrap_or("live")
        .to_lowercase();
    let result = match purpose.as_str() {
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
        _ => {
            return Err(HttpError::Signal(cheetah_signal_types::SignalError::new(
                cheetah_signal_types::SignalErrorKind::InvalidArgument,
                "unsupported media purpose",
            )));
        }
    };
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::to_value(result).map_err(HttpError::from)?),
    ))
}

pub async fn get_session(
    Path(id): Path<String>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("viewer")?;
    let _media_session_id = id.parse::<MediaSessionId>().map_err(HttpError::from)?;
    Err(HttpError::NotImplemented(
        "get_session not implemented".to_string(),
    ))
}

pub async fn stop_session(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    Json(_body): Json<serde_json::Value>,
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
    Ok(Json(serde_json::to_value(result).map_err(HttpError::from)?))
}

pub async fn control_session(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    Json(body): Json<serde_json::Value>,
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
    Ok(Json(serde_json::to_value(result).map_err(HttpError::from)?))
}

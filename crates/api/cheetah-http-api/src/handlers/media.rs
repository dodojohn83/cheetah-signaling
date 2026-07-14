//! Media session HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use cheetah_signal_application::dto::{
    ControlPlaybackRequest, StartLiveRequest, StartPlaybackRequest, StartTalkRequest,
    StopLiveRequest,
};
use cheetah_signal_types::{MediaSessionId, Page};
use std::sync::Arc;

pub async fn list_sessions(
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    _ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    Ok(Json(Page::new(Vec::new())))
}

pub async fn create_session(
    State(state): State<Arc<ApiState>>,
    _ctx: ApiRequestContext,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), HttpError> {
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let purpose = body
        .get("purpose")
        .and_then(|v| v.as_str())
        .unwrap_or("live");
    let result = match purpose {
        "live" | "LIVE" => {
            let request: StartLiveRequest =
                serde_json::from_value(body).map_err(HttpError::from)?;
            state
                .media_service
                .start_live(&_ctx.0, &mut *uow, request)
                .await
                .map_err(HttpError::from)?
        }
        "playback" => {
            let request: StartPlaybackRequest =
                serde_json::from_value(body).map_err(HttpError::from)?;
            state
                .media_service
                .start_playback(&_ctx.0, &mut *uow, request)
                .await
                .map_err(HttpError::from)?
        }
        "talk" => {
            let request: StartTalkRequest =
                serde_json::from_value(body).map_err(HttpError::from)?;
            state
                .media_service
                .start_talk(&_ctx.0, &mut *uow, request)
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
    _ctx: ApiRequestContext,
) -> Result<Json<serde_json::Value>, HttpError> {
    let _media_session_id = id.parse::<MediaSessionId>().map_err(HttpError::from)?;
    Err(HttpError::NotImplemented(
        "get_session not implemented".to_string(),
    ))
}

pub async fn stop_session(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    _ctx: ApiRequestContext,
    Json(_body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let media_session_id = id.parse::<MediaSessionId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let request = StopLiveRequest {
        media_session_id: media_session_id.to_string(),
        idempotency_key: uuid::Uuid::now_v7().to_string(),
    };
    let result = state
        .media_service
        .stop_live(&_ctx.0, &mut *uow, request)
        .await
        .map_err(HttpError::from)?;
    Ok(Json(serde_json::to_value(result).map_err(HttpError::from)?))
}

pub async fn control_session(
    Path(id): Path<String>,
    State(state): State<Arc<ApiState>>,
    _ctx: ApiRequestContext,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let media_session_id = id.parse::<MediaSessionId>().map_err(HttpError::from)?;
    let mut request: ControlPlaybackRequest =
        serde_json::from_value(body).map_err(HttpError::from)?;
    request.media_session_id = media_session_id.to_string();
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let result = state
        .media_service
        .control_playback(&_ctx.0, &mut *uow, request)
        .await
        .map_err(HttpError::from)?;
    Ok(Json(serde_json::to_value(result).map_err(HttpError::from)?))
}

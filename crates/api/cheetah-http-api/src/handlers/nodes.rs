//! Node and media node HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError};
use axum::{
    Json,
    extract::{Query, State},
};
use cheetah_signal_types::Page;

use std::sync::Arc;

use crate::ListQuery;

pub async fn list_nodes(
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    _ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    Ok(Json(Page::new(Vec::new())))
}

pub async fn list_media_nodes(
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    _ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    Ok(Json(Page::new(Vec::new())))
}

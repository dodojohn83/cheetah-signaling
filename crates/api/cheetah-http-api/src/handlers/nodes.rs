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
    ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    ctx.require_scope("viewer")?;
    Err(HttpError::NotImplemented(
        "node list pagination is not yet implemented".to_string(),
    ))
}

pub async fn list_media_nodes(
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    ctx.require_scope("viewer")?;
    Err(HttpError::NotImplemented(
        "media node list pagination is not yet implemented".to_string(),
    ))
}

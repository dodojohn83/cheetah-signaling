//! Tenant HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, ListQuery};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use cheetah_signal_types::Page;
use std::sync::Arc;

pub async fn list_tenants(
    Query(_query): Query<ListQuery>,
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    ctx.require_scope("viewer")?;
    Ok(Json(Page::new(Vec::new())))
}

pub async fn create_tenant(
    State(_state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    Json(_body): Json<serde_json::Value>,
) -> Result<StatusCode, HttpError> {
    ctx.require_scope("system_admin")?;
    Err(HttpError::NotImplemented(
        "tenant creation is not implemented".to_string(),
    ))
}

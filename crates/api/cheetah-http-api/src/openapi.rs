//! OpenAPI spec serving.

use crate::{ApiRequestContext, ApiState, HttpError};
use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

const OPENAPI_YAML: &str = include_str!("../openapi/signaling-v1.yaml");

/// Serves the OpenAPI specification as JSON.
pub async fn serve_json(
    _state: State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Response, HttpError> {
    ctx.require_scope("viewer")?;
    let value: serde_json::Value = serde_yaml::from_str(OPENAPI_YAML)
        .map_err(|e| HttpError::internal(format!("failed to parse openapi spec: {e}")))?;
    let body = serde_json::to_string(&value)
        .map_err(|e| HttpError::internal(format!("failed to serialize openapi spec: {e}")))?;
    Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
}

/// Serves the OpenAPI specification as YAML.
pub async fn serve_yaml(
    _state: State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Response, HttpError> {
    ctx.require_scope("viewer")?;
    Ok(([(header::CONTENT_TYPE, "application/yaml")], OPENAPI_YAML).into_response())
}

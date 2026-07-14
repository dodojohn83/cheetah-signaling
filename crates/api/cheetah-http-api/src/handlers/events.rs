//! Server-Sent Events (SSE) HTTP handler.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError};
use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use futures::stream::Stream;
use serde::Deserialize;
use std::sync::Arc;

/// Query parameters for the event stream.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct EventQuery {
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub event_type: Option<String>,
}

pub async fn event_stream(
    Query(_query): Query<EventQuery>,
    State(_state): State<Arc<ApiState>>,
    _ctx: ApiRequestContext,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, HttpError> {
    let stream = futures::stream::empty();
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

//! Server-Sent Events (SSE) HTTP handler.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError};
use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;

/// Query parameters for the event stream.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct EventQuery {
    /// Resume after this cursor.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Filter by device identifier.
    #[serde(default)]
    pub device_id: Option<String>,
    /// Filter by event type name.
    #[serde(default)]
    pub event_type: Option<String>,
}

pub async fn event_stream(
    Query(query): Query<EventQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, HttpError> {
    ctx.require_scope("viewer")?;
    let tenant_filter = Some(ctx.tenant_id.to_string());
    let device_filter = query.device_id;
    let type_filter = query.event_type;
    let start_cursor = query
        .cursor
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| state.event_cache.latest_cursor());

    let cache = state.event_cache.clone();
    let (tx, rx) = tokio::sync::mpsc::channel(64);

    tokio::spawn(async move {
        let mut watch = cache.watch();
        let mut last = start_cursor;
        loop {
            let events = cache.events_after(
                last,
                tenant_filter.as_deref(),
                device_filter.as_deref(),
                type_filter.as_deref(),
            );
            for ev in events {
                let sse_event = Event::default()
                    .id(ev.cursor.to_string())
                    .event(ev.event_type)
                    .data(ev.json);
                if tx.send(sse_event).await.is_err() {
                    return;
                }
                last = ev.cursor;
            }
            if watch.changed().await.is_err() {
                break;
            }
            let cursor = *watch.borrow();
            if cursor <= last {
                continue;
            }
        }
    });

    let stream =
        ReceiverStream::new(rx).map(|event: Event| Ok::<_, std::convert::Infallible>(event));
    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

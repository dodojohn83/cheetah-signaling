//! Server-Sent Events (SSE) HTTP handler.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError};
use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use cheetah_signal_types::{SignalError, SignalErrorKind};
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

/// Maximum byte length of an SSE `device_id` query parameter.
const MAX_EVENT_DEVICE_ID_BYTES: usize = 128;
/// Maximum byte length of an SSE `event_type` query parameter.
const MAX_EVENT_TYPE_BYTES: usize = 256;

impl EventQuery {
    /// Validates query parameter lengths to prevent unbounded allocation on
    /// malformed or malicious SSE requests.
    fn validate(&self) -> Result<(), HttpError> {
        if let Some(device_id) = &self.device_id
            && device_id.len() > MAX_EVENT_DEVICE_ID_BYTES
        {
            return Err(HttpError::Signal(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "device_id query parameter exceeds maximum length",
            )));
        }
        if let Some(event_type) = &self.event_type
            && event_type.len() > MAX_EVENT_TYPE_BYTES
        {
            return Err(HttpError::Signal(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "event_type query parameter exceeds maximum length",
            )));
        }
        Ok(())
    }
}

pub async fn event_stream(
    Query(query): Query<EventQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, HttpError> {
    ctx.require_scope("viewer")?;
    query.validate()?;
    let tenant_filter = Some(ctx.tenant_id.to_string());
    let device_filter = query.device_id;
    let type_filter = query.event_type;
    let start_cursor = resolve_start_cursor(query.cursor, state.event_cache.latest_cursor())?;

    let cache = state.event_cache.clone();
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let cancel = state.cancel.child_token();

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
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tx.closed() => break,
                changed = watch.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let cursor = *watch.borrow();
                    if cursor == 0 || cursor == last {
                        continue;
                    }
                    if cursor < last {
                        // Cursor wrapped around after overflow; reset and
                        // deliver new events from the beginning of the buffer.
                        last = 0;
                        continue;
                    }
                }
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

/// Maximum byte length of an SSE cursor query parameter. `u64` needs at most
/// 20 decimal digits, so anything far longer is malformed and wastes CPU
/// parsing.
const MAX_EVENT_CURSOR_BYTES: usize = 32;

fn resolve_start_cursor(cursor: Option<String>, latest: u64) -> Result<u64, HttpError> {
    match cursor.as_deref() {
        Some(s) if !s.is_empty() => {
            if s.len() > MAX_EVENT_CURSOR_BYTES {
                return Err(HttpError::Signal(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "cursor too long",
                )));
            }
            s.parse::<u64>().map_err(|_| {
                HttpError::Signal(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "cursor must be a non-negative integer",
                ))
            })
        }
        _ => Ok(latest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_start_cursor_uses_latest_when_missing() {
        match resolve_start_cursor(None, 42) {
            Ok(cursor) => assert_eq!(cursor, 42),
            Err(_) => panic!("expected latest cursor"),
        }
    }

    #[test]
    fn resolve_start_cursor_uses_latest_for_empty_string() {
        match resolve_start_cursor(Some("".into()), 42) {
            Ok(cursor) => assert_eq!(cursor, 42),
            Err(_) => panic!("expected latest cursor for empty string"),
        }
    }

    #[test]
    fn resolve_start_cursor_parses_valid_cursor() {
        match resolve_start_cursor(Some("7".into()), 42) {
            Ok(cursor) => assert_eq!(cursor, 7),
            Err(_) => panic!("expected parsed cursor"),
        }
    }

    #[test]
    fn resolve_start_cursor_rejects_non_numeric_cursor() {
        match resolve_start_cursor(Some("abc".into()), 42) {
            Err(err) => assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST),
            Ok(_) => panic!("expected invalid cursor error"),
        }
    }
    #[test]
    fn resolve_start_cursor_rejects_oversized_cursor() {
        match resolve_start_cursor(Some("0".repeat(MAX_EVENT_CURSOR_BYTES + 1)), 42) {
            Err(err) => assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST),
            Ok(_) => panic!("expected oversized cursor error"),
        }
    }

    #[test]
    fn event_query_rejects_oversized_device_id() {
        let query = EventQuery {
            device_id: Some("x".repeat(MAX_EVENT_DEVICE_ID_BYTES + 1)),
            ..Default::default()
        };
        assert!(query.validate().is_err());
    }

    #[test]
    fn event_query_rejects_oversized_event_type() {
        let query = EventQuery {
            event_type: Some("x".repeat(MAX_EVENT_TYPE_BYTES + 1)),
            ..Default::default()
        };
        assert!(query.validate().is_err());
    }

    #[test]
    fn event_query_accepts_defaults() {
        let query = EventQuery::default();
        assert!(query.validate().is_ok());
    }
}

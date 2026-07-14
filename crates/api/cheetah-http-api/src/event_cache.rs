//! Bounded in-memory cache for SSE event streams.

#![allow(missing_docs)]

use cheetah_message_api::{EventEnvelope, decode_event};
use cheetah_signal_types::ResourceId;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// A single cached event with a monotonic cursor and pre-serialized payload.
#[derive(Clone, Debug)]
pub struct CachedEvent {
    pub cursor: u64,
    pub tenant_id: String,
    pub device_id: Option<String>,
    pub event_type: String,
    pub json: String,
}

struct Inner {
    next_cursor: u64,
    buffer: VecDeque<CachedEvent>,
}

/// Bounded cache that keeps the most recent events for slow consumers.
pub struct EventCache {
    capacity: usize,
    inner: Mutex<Inner>,
    tx: watch::Sender<u64>,
}

impl std::fmt::Debug for EventCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventCache")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl EventCache {
    /// Creates a new cache with the given capacity and returns an `Arc` handle.
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _rx) = watch::channel(0u64);
        Arc::new(Self {
            capacity,
            inner: Mutex::new(Inner {
                next_cursor: 1,
                buffer: VecDeque::new(),
            }),
            tx,
        })
    }

    /// Decodes and stores an event envelope, returning the assigned cursor.
    pub fn push(&self, envelope: &EventEnvelope) -> Result<u64, cheetah_message_api::BusError> {
        let event = decode_event(envelope)?;
        let tenant_id = event.tenant_id.to_string();
        let device_id = match &event.aggregate_ref.id {
            ResourceId::Device(id) => Some(id.to_string()),
            _ => None,
        };
        let payload = serde_json::to_value(&event.payload).unwrap_or(Value::Null);
        let event_type = match payload {
            Value::Object(map) => map
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            _ => "unknown".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap_or_default();

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cursor = inner.next_cursor;
        let new_cursor = match cursor.checked_add(1) {
            Some(c) => c,
            None => {
                tracing::warn!("event cursor overflow; clearing bounded cache");
                inner.buffer.clear();
                1
            }
        };
        inner.next_cursor = new_cursor;
        inner.buffer.push_back(CachedEvent {
            cursor,
            tenant_id,
            device_id,
            event_type,
            json,
        });
        while inner.buffer.len() > self.capacity {
            inner.buffer.pop_front();
        }
        let _ = self.tx.send(cursor);
        Ok(cursor)
    }

    /// Returns the most recently assigned cursor.
    pub fn latest_cursor(&self) -> u64 {
        *self.tx.borrow()
    }

    /// Returns a receiver that is notified when the cursor advances.
    pub fn watch(&self) -> watch::Receiver<u64> {
        self.tx.subscribe()
    }

    /// Returns events with cursor greater than `after` and matching the filters.
    pub fn events_after(
        &self,
        after: u64,
        tenant_filter: Option<&str>,
        device_filter: Option<&str>,
        type_filter: Option<&str>,
    ) -> Vec<CachedEvent> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner
            .buffer
            .iter()
            .filter(|ev| ev.cursor > after)
            .filter(|ev| tenant_filter.is_none_or(|t| ev.tenant_id == t))
            .filter(|ev| device_filter.is_none_or(|d| ev.device_id.as_deref() == Some(d)))
            .filter(|ev| type_filter.is_none_or(|t| ev.event_type == t))
            .cloned()
            .collect()
    }
}

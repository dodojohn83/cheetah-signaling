//! Webhook dispatcher and store (stub).

use std::sync::Arc;

/// Shared webhook state.
#[derive(Clone, Debug, Default)]
pub struct WebhookStore {}

impl WebhookStore {
    /// Creates an empty webhook store.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {})
    }
}

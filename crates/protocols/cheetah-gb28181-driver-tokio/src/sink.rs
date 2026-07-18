//! Event sink abstraction for GB28181 domain events.

use cheetah_gb28181_module::Gb28181Event;

/// Receives [`Gb28181Event`]s produced by the driver.
///
/// Implementations are expected to be non-blocking and should not propagate
/// backpressure into the UDP receive loop.
pub trait EventSink: Send + Sync {
    /// Emits a domain event.
    fn emit(&self, event: Gb28181Event);
}

/// An event sink that discards all events.
#[derive(Debug)]
pub struct NoOpEventSink;

impl EventSink for NoOpEventSink {
    fn emit(&self, _event: Gb28181Event) {}
}

impl EventSink for tokio::sync::mpsc::Sender<Gb28181Event> {
    fn emit(&self, event: Gb28181Event) {
        if let Err(e) = self.try_send(event) {
            tracing::warn!(error = %e, "event sink full; dropping GB28181 event");
        }
    }
}

//! Generic event sink abstraction for GB28181 access machines.
//!
//! The sink is generic over the event type so the driver can be driven by any
//! [`cheetah_gb28181_core::GbAccessMachine`] without depending on a concrete
//! event enum.

/// Receives events produced by a GB28181 access driver.
///
/// Implementations are expected to be non-blocking and should not propagate
/// backpressure into the UDP receive loop.
pub trait EventSink<E: Send>: Send + Sync {
    /// Emits an event.
    fn emit(&self, event: E);
}

/// An event sink that discards all events.
#[derive(Debug)]
pub struct NoOpEventSink;

impl<E: Send> EventSink<E> for NoOpEventSink {
    fn emit(&self, _event: E) {}
}

impl<E: Send> EventSink<E> for tokio::sync::mpsc::Sender<E> {
    fn emit(&self, event: E) {
        if let Err(e) = self.try_send(event) {
            tracing::warn!(error = %e, "event sink full; dropping GB28181 event");
        }
    }
}

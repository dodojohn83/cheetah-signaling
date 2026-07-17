//! A mock host for plugin contract tests.
//!
//! `MockHost` implements [`DeviceSink`] and [`CommandSource`] from
//! `cheetah_plugin_sdk`, recording every emitted event and allowing tests to
//! queue commands for the driver under test.

use async_trait::async_trait;
use cheetah_plugin_sdk::{CommandSource, DeviceSink, DriverCommand, PluginError, ProtocolEvent};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::{Mutex as TokioMutex, mpsc};

/// Bounded capacity for the command queue shared between tests and the mock host.
const COMMAND_QUEUE_CAPACITY: usize = 64;

/// In-memory event/command recorder used to validate plugin driver behavior.
#[derive(Clone, Debug)]
pub struct MockHost {
    events: Arc<Mutex<Vec<ProtocolEvent>>>,
    command_tx: mpsc::Sender<DriverCommand>,
    command_rx: Arc<TokioMutex<Option<mpsc::Receiver<DriverCommand>>>>,
}

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MockHost {
    /// Create a new mock host with an empty event log and an empty command queue.
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            command_tx,
            command_rx: Arc::new(TokioMutex::new(Some(command_rx))),
        }
    }

    /// Enqueue a command that will be returned by [`CommandSource::next_command`].
    ///
    /// Returns an error if the bounded command queue is full.
    pub fn push_command(&self, command: DriverCommand) -> Result<(), PluginError> {
        self.command_tx
            .try_send(command)
            .map_err(|_| PluginError::Driver("mock host command queue is full".to_string()))
    }

    /// Return all events emitted by the driver since the last call.
    pub fn take_events(&self) -> Vec<ProtocolEvent> {
        let mut guard = lock(&self.events);
        std::mem::take(&mut *guard)
    }

    /// Return the number of events currently recorded.
    pub fn event_count(&self) -> usize {
        let guard = lock(&self.events);
        guard.len()
    }
}

/// Acquire a mutex guard, recovering from poisoning if the previous holder panicked.
///
/// Test utilities should not bring down the test runner because an unrelated
/// task panicked while holding this lock.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[async_trait]
impl DeviceSink for MockHost {
    async fn emit_event(&self, event: ProtocolEvent) -> Result<(), PluginError> {
        let mut guard = lock(&self.events);
        guard.push(event);
        Ok(())
    }
}

#[async_trait]
impl CommandSource for MockHost {
    async fn next_command(&self) -> Result<Option<DriverCommand>, PluginError> {
        // Take the receiver out of the shared slot and wrap it in a guard that
        // returns the receiver on drop, so cancellation does not lose it.
        let receiver = {
            let mut guard = self.command_rx.lock().await;
            guard.take().ok_or_else(|| {
                PluginError::Driver("mock host command receiver is already in use".to_string())
            })?
        };

        let mut guard = ReceiverGuard {
            slot: self.command_rx.clone(),
            receiver: Some(receiver),
        };

        let command = guard.recv().await;
        Ok(command)
    }
}

/// Guard that returns the command receiver to its shared slot when dropped.
///
/// This makes `next_command` cancellation-safe: if the future is dropped while
/// awaiting `recv`, the receiver is still restored.
struct ReceiverGuard {
    slot: Arc<TokioMutex<Option<mpsc::Receiver<DriverCommand>>>>,
    receiver: Option<mpsc::Receiver<DriverCommand>>,
}

impl ReceiverGuard {
    async fn recv(&mut self) -> Option<DriverCommand> {
        if let Some(ref mut rx) = self.receiver {
            rx.recv().await
        } else {
            None
        }
    }
}

impl Drop for ReceiverGuard {
    fn drop(&mut self) {
        if let Some(receiver) = self.receiver.take() {
            // Use a synchronous try-lock; if the async mutex is held by another
            // task the receiver cannot be restored here, which is acceptable in
            // a test-only utility.
            if let Ok(mut guard) = self.slot.try_lock()
                && guard.is_none()
            {
                *guard = Some(receiver);
            }
        }
    }
}

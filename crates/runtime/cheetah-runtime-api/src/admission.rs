//! Admission control port.

use crate::{DeviceKey, RuntimeError, RuntimeMessage};

/// Port for sending messages into a runtime.
pub trait AdmissionController: Send + Sync + 'static {
    /// Sends a message to the shard responsible for the given device key.
    ///
    /// Returns `Overloaded` if the target mailbox is full, or `Shutdown` if the
    /// runtime has been dropped.
    fn send(&self, device_key: DeviceKey, message: RuntimeMessage) -> Result<(), RuntimeError>;

    /// Returns the number of shards.
    fn shard_count(&self) -> usize;

    /// Returns the maximum number of pending messages per shard.
    fn max_pending(&self) -> usize;
}

//! Bounded admission controller for shard mailboxes.

use std::sync::Arc;

use cheetah_runtime_api::{
    AdmissionController as AdmissionControllerTrait, DeviceKey, RuntimeError, RuntimeMessage,
    RuntimeMetrics, ShardRouter,
};
use tokio::sync::mpsc;

/// Tokio implementation of the admission controller.
#[derive(Clone)]
pub struct AdmissionController {
    router: ShardRouter,
    senders: Arc<Vec<mpsc::Sender<RuntimeMessage>>>,
    max_pending: usize,
    metrics: Arc<RuntimeMetrics>,
}

impl std::fmt::Debug for AdmissionController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionController")
            .field("shard_count", &self.senders.len())
            .field("max_pending", &self.max_pending)
            .finish_non_exhaustive()
    }
}

impl AdmissionController {
    /// Creates a new admission controller.
    pub(crate) fn new(
        router: ShardRouter,
        senders: Arc<Vec<mpsc::Sender<RuntimeMessage>>>,
        max_pending: usize,
        metrics: Arc<RuntimeMetrics>,
    ) -> Self {
        Self {
            router,
            senders,
            max_pending,
            metrics,
        }
    }

    /// Returns the shard senders for shutdown.
    pub(crate) fn senders(&self) -> &[mpsc::Sender<RuntimeMessage>] {
        &self.senders[..]
    }

    /// Returns the current occupancy of each shard mailbox, indexed by shard.
    ///
    /// Depth is derived from the bounded MPSC channel as
    /// `max_capacity - available_capacity`, giving a per-shard gauge whose
    /// label cardinality is fixed by the (bounded) shard count.
    pub fn shard_mailbox_depths(&self) -> Vec<u64> {
        self.senders
            .iter()
            .map(|sender| {
                let used = sender.max_capacity().saturating_sub(sender.capacity());
                used as u64
            })
            .collect()
    }
}

impl AdmissionControllerTrait for AdmissionController {
    fn send(&self, device_key: DeviceKey, message: RuntimeMessage) -> Result<(), RuntimeError> {
        let index = self.router.route(device_key);
        let sender = self
            .senders
            .get(index)
            .ok_or_else(|| RuntimeError::Internal("invalid shard index".into()))?;
        match sender.try_send(message) {
            Ok(()) => {
                self.metrics.record_message_enqueued();
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.metrics.record_message_rejected();
                Err(RuntimeError::Overloaded)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(RuntimeError::Shutdown),
        }
    }

    fn shard_count(&self) -> usize {
        self.senders.len()
    }

    fn max_pending(&self) -> usize {
        self.max_pending
    }
}

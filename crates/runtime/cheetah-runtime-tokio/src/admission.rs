//! Bounded admission controller for shard mailboxes.

use std::sync::Arc;

use cheetah_runtime_api::{
    AdmissionController as AdmissionControllerTrait, DeviceKey, RuntimeError, RuntimeMessage,
    RuntimeMetrics, ShardRouter,
};
use cheetah_signal_types::Clock;
use cheetah_signal_types::admission::{DeadLetterReason, TrafficClass};
use tokio::sync::mpsc;

use crate::admission_policy::{AdmissionOutcome, AdmissionPolicy, AdmissionTicket, PreAdmitAction};

/// Result of routing a message to a shard mailbox.
enum RouteSend {
    /// The message was enqueued.
    Sent,
    /// The mailbox was full; the message is returned.
    Full(RuntimeMessage),
    /// The mailbox is closed.
    Closed,
}

/// Tokio implementation of the admission controller.
#[derive(Clone)]
pub struct AdmissionController {
    router: ShardRouter,
    senders: Arc<Vec<mpsc::Sender<RuntimeMessage>>>,
    max_pending: usize,
    metrics: Arc<RuntimeMetrics>,
    clock: Arc<dyn Clock>,
    policy: Option<Arc<AdmissionPolicy>>,
}

impl std::fmt::Debug for AdmissionController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionController")
            .field("shard_count", &self.senders.len())
            .field("max_pending", &self.max_pending)
            .field("policy_enabled", &self.policy.is_some())
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
        clock: Arc<dyn Clock>,
        policy: Option<Arc<AdmissionPolicy>>,
    ) -> Self {
        Self {
            router,
            senders,
            max_pending,
            metrics,
            clock,
            policy,
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

    fn now_ms(&self) -> i64 {
        self.clock.now_monotonic().as_millis()
    }

    fn aggregate_depth(&self) -> u64 {
        self.shard_mailbox_depths().iter().sum()
    }

    fn route_send(&self, device_key: DeviceKey, message: RuntimeMessage) -> RouteSend {
        let index = self.router.route(device_key);
        match self.senders.get(index) {
            Some(sender) => match sender.try_send(message) {
                Ok(()) => RouteSend::Sent,
                Err(mpsc::error::TrySendError::Full(m)) => RouteSend::Full(m),
                Err(mpsc::error::TrySendError::Closed(_)) => RouteSend::Closed,
            },
            None => RouteSend::Closed,
        }
    }

    /// Admits a classified message through the full admission policy: per-source
    /// and per-method rate limiting, priority-aware overload shedding,
    /// coalescing of redundant keepalive/position events and bounded
    /// dead-lettering when the target mailbox is full.
    ///
    /// When no policy is configured this behaves like [`AdmissionControllerTrait::send`].
    pub fn admit(
        &self,
        ticket: AdmissionTicket,
        message: RuntimeMessage,
    ) -> Result<AdmissionOutcome, RuntimeError> {
        let Some(policy) = self.policy.clone() else {
            self.send(ticket.device_key, message)?;
            return Ok(AdmissionOutcome::Admitted);
        };

        let now = self.now_ms();
        let depth = self.aggregate_depth();
        let decision = policy.pre_admit(now, depth, ticket);
        if decision.entered_overload {
            self.metrics.record_backlog_overload_transition();
        }

        match decision.action {
            PreAdmitAction::Admit => match self.route_send(ticket.device_key, message) {
                RouteSend::Sent => {
                    self.metrics.record_message_enqueued();
                    Ok(AdmissionOutcome::Admitted)
                }
                RouteSend::Full(m) => {
                    policy.dead_letter(ticket, m, DeadLetterReason::Overloaded, now);
                    self.metrics.record_message_rejected();
                    self.metrics.record_message_dead_lettered();
                    Ok(AdmissionOutcome::DeadLettered)
                }
                RouteSend::Closed => Err(RuntimeError::Shutdown),
            },
            PreAdmitAction::Coalesce => {
                self.metrics.record_message_coalesced();
                Ok(AdmissionOutcome::Coalesced)
            }
            PreAdmitAction::RateLimit => {
                policy.dead_letter(ticket, message, DeadLetterReason::RateLimited, now);
                self.metrics.record_message_rate_limited();
                self.metrics.record_message_dead_lettered();
                Ok(AdmissionOutcome::RateLimited)
            }
            PreAdmitAction::Shed => {
                policy.dead_letter(ticket, message, DeadLetterReason::PriorityShed, now);
                self.metrics.record_message_shed();
                self.metrics.record_message_dead_lettered();
                Ok(AdmissionOutcome::PriorityShed)
            }
        }
    }

    /// Releases a coalescible pending event once processed so that a subsequent
    /// equivalent event is admitted rather than coalesced.
    pub fn release_coalescible(&self, device_key: DeviceKey, class: TrafficClass) {
        if let Some(policy) = self.policy.as_ref() {
            policy.release_coalescible(device_key, class);
        }
    }

    /// Attempts to redrive up to `max` dead-lettered messages back onto their
    /// shard mailboxes. Redrive only proceeds once the runtime has recovered
    /// from overload. Returns the number of messages successfully re-admitted.
    pub fn redrive(&self, max: usize) -> Result<usize, RuntimeError> {
        let Some(policy) = self.policy.clone() else {
            return Ok(0);
        };
        let Some(batch) = policy.take_redrive_batch(max) else {
            return Ok(0);
        };

        let mut admitted = 0u64;
        for entry in batch {
            let device_key = entry.payload.ticket.device_key;
            match self.route_send(device_key, entry.payload.message.clone()) {
                RouteSend::Sent => {
                    admitted += 1;
                    self.metrics.record_message_enqueued();
                }
                RouteSend::Full(_) => {
                    policy.requeue_failed_redrive(entry);
                }
                RouteSend::Closed => return Err(RuntimeError::Shutdown),
            }
        }
        self.metrics.record_messages_redriven(admitted);
        Ok(admitted as usize)
    }

    /// Returns the configured admission policy, if any.
    pub fn policy(&self) -> Option<&Arc<AdmissionPolicy>> {
        self.policy.as_ref()
    }
}

impl AdmissionControllerTrait for AdmissionController {
    fn send(&self, device_key: DeviceKey, message: RuntimeMessage) -> Result<(), RuntimeError> {
        match self.route_send(device_key, message) {
            RouteSend::Sent => {
                self.metrics.record_message_enqueued();
                Ok(())
            }
            RouteSend::Full(_) => {
                self.metrics.record_message_rejected();
                Err(RuntimeError::Overloaded)
            }
            RouteSend::Closed => Err(RuntimeError::Shutdown),
        }
    }

    fn shard_count(&self) -> usize {
        self.senders.len()
    }

    fn max_pending(&self) -> usize {
        self.max_pending
    }
}

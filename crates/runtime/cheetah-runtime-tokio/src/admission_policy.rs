//! Composed admission policy: rate limiting, coalescing, priority shedding,
//! dead-lettering and backlog recovery.
//!
//! This is the pure decision engine used by the [`crate::admission::AdmissionController`].
//! It owns no channels and performs no I/O; it only decides what should happen
//! to an admitted message and remembers dead-lettered work for redrive. All
//! state is bounded by [`AdmissionPolicyConfig`].

use std::sync::Mutex;

use cheetah_runtime_api::{AdmissionPolicyConfig, DeviceKey, RuntimeMessage};
use cheetah_signal_types::admission::{
    BacklogController, CoalesceDecision, Coalescer, DeadLetterEntry, DeadLetterQueue,
    DeadLetterReason, KeyedRateLimiter, Priority, TokenBucketConfig, TrafficClass,
};

/// Maximum number of redrive attempts before a message is permanently dropped.
const MAX_REDRIVE_ATTEMPTS: u32 = 8;

/// Describes an admission request: who sent it, what kind of work it is, and
/// which device it targets.
#[derive(Clone, Copy, Debug)]
pub struct AdmissionTicket {
    /// Opaque source identity (e.g. a hash of tenant + remote endpoint) used
    /// for per-source rate limiting.
    pub source_id: u64,
    /// Traffic classification used for per-method limiting, priority and
    /// coalescing.
    pub class: TrafficClass,
    /// Target device.
    pub device_key: DeviceKey,
}

/// Outcome of an admission decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionOutcome {
    /// The message was admitted to a shard mailbox.
    Admitted,
    /// The message was coalesced into a pending equivalent event.
    Coalesced,
    /// The message was rejected by rate limiting and dead-lettered.
    RateLimited,
    /// The message was shed because it was low priority during overload and
    /// dead-lettered.
    PriorityShed,
    /// The target mailbox was full; the message was dead-lettered.
    DeadLettered,
}

/// A message retained for redrive, together with its admission ticket.
#[derive(Clone, Debug)]
pub struct PendingAdmission {
    /// The original admission ticket.
    pub ticket: AdmissionTicket,
    /// The original message.
    pub message: RuntimeMessage,
}

/// Action the controller should take for a pre-admitted message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreAdmitAction {
    /// Attempt to enqueue the message on its shard mailbox.
    Admit,
    /// Drop the message as a redundant coalesced event.
    Coalesce,
    /// Reject the message due to rate limiting.
    RateLimit,
    /// Shed the message as low-priority work during overload.
    Shed,
}

/// Result of [`AdmissionPolicy::pre_admit`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreAdmitDecision {
    /// The action to take.
    pub action: PreAdmitAction,
    /// Whether this observation transitioned the runtime into overload.
    pub entered_overload: bool,
}

struct PolicyState {
    rate_limiter: KeyedRateLimiter<(u64, TrafficClass)>,
    coalescer: Coalescer<(DeviceKey, TrafficClass)>,
    dead_letter: DeadLetterQueue<PendingAdmission>,
    backlog: BacklogController,
}

/// Thread-safe admission policy engine.
#[derive(Debug)]
pub struct AdmissionPolicy {
    state: Mutex<PolicyState>,
}

impl std::fmt::Debug for PolicyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyState")
            .field("rate_keys", &self.rate_limiter.tracked_keys())
            .field("coalescer_pending", &self.coalescer.pending_len())
            .field("dead_letter_len", &self.dead_letter.len())
            .field("backlog_state", &self.backlog.state())
            .finish()
    }
}

impl AdmissionPolicy {
    /// Builds a policy from configuration.
    pub fn new(config: &AdmissionPolicyConfig) -> Self {
        let rate_config = TokenBucketConfig {
            capacity_tokens: config.rate_capacity_tokens,
            refill_tokens_per_sec: config.rate_refill_tokens_per_sec,
        };
        let state = PolicyState {
            rate_limiter: KeyedRateLimiter::new(rate_config, config.rate_max_keys),
            coalescer: Coalescer::new(config.coalesce_max_tracked),
            dead_letter: DeadLetterQueue::new(config.dead_letter_capacity),
            backlog: BacklogController::new(
                config.backlog_high_watermark,
                config.backlog_low_watermark,
            ),
        };
        Self {
            state: Mutex::new(state),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PolicyState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Evaluates admission for a ticket given the current aggregate backlog
    /// depth. This applies backlog, rate-limit and coalescing bookkeeping and
    /// returns the action the controller should take. No dead-lettering or
    /// channel send happens here.
    pub(crate) fn pre_admit(
        &self,
        now_ms: i64,
        aggregate_depth: u64,
        ticket: AdmissionTicket,
    ) -> PreAdmitDecision {
        let mut state = self.lock();
        let observation = state.backlog.observe(aggregate_depth);
        let entered_overload = observation.entered_overload;

        if state.backlog.shed_low_priority() && ticket.class.priority() == Priority::Low {
            return PreAdmitDecision {
                action: PreAdmitAction::Shed,
                entered_overload,
            };
        }

        if !state
            .rate_limiter
            .try_acquire((ticket.source_id, ticket.class), now_ms)
        {
            return PreAdmitDecision {
                action: PreAdmitAction::RateLimit,
                entered_overload,
            };
        }

        if ticket.class.is_coalescible() {
            match state.coalescer.observe((ticket.device_key, ticket.class)) {
                CoalesceDecision::Coalesced => {
                    return PreAdmitDecision {
                        action: PreAdmitAction::Coalesce,
                        entered_overload,
                    };
                }
                CoalesceDecision::Admit => {}
            }
        }

        PreAdmitDecision {
            action: PreAdmitAction::Admit,
            entered_overload,
        }
    }

    /// Records a dead-lettered message for later redrive.
    ///
    /// If the ticket was already observed by the coalescer, release the pending
    /// key so a later equivalent event is not permanently coalesced away.
    pub(crate) fn dead_letter(
        &self,
        ticket: AdmissionTicket,
        message: RuntimeMessage,
        reason: DeadLetterReason,
        now_ms: i64,
    ) {
        let device_key = ticket.device_key;
        let class = ticket.class;
        let mut state = self.lock();
        state
            .dead_letter
            .push(PendingAdmission { ticket, message }, reason, now_ms);
        if class.is_coalescible() {
            state.coalescer.release(&(device_key, class));
        }
    }

    /// Releases a coalescible pending event once it has been processed, so a
    /// subsequent equivalent event will be admitted rather than coalesced.
    pub fn release_coalescible(&self, device_key: DeviceKey, class: TrafficClass) {
        if class.is_coalescible() {
            self.lock().coalescer.release(&(device_key, class));
        }
    }

    /// Drains up to `max` dead-lettered messages for redrive, but only when the
    /// runtime is no longer overloaded. Returns `None` while overloaded.
    pub(crate) fn take_redrive_batch(
        &self,
        max: usize,
    ) -> Option<Vec<DeadLetterEntry<PendingAdmission>>> {
        let mut state = self.lock();
        if state.backlog.shed_low_priority() {
            return None;
        }
        let batch = state.dead_letter.drain(max);
        if batch.is_empty() { None } else { Some(batch) }
    }

    /// Re-enqueues a dead-letter entry whose redrive failed, incrementing its
    /// attempt count. Entries that exceed [`MAX_REDRIVE_ATTEMPTS`] are dropped.
    ///
    /// Returns `true` if the entry was requeued, `false` if it was permanently
    /// dropped.
    pub(crate) fn requeue_failed_redrive(
        &self,
        mut entry: DeadLetterEntry<PendingAdmission>,
    ) -> bool {
        entry.attempts = entry.attempts.saturating_add(1);
        if entry.attempts >= MAX_REDRIVE_ATTEMPTS {
            return false;
        }
        entry.reason = DeadLetterReason::Overloaded;
        self.lock().dead_letter.push_entry(entry);
        true
    }

    /// Returns the current number of dead-lettered messages awaiting redrive.
    pub fn dead_letter_len(&self) -> usize {
        self.lock().dead_letter.len()
    }

    /// Returns whether the runtime is currently in the backlog overload state.
    pub fn is_overloaded(&self) -> bool {
        self.lock().backlog.shed_low_priority()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheetah_signal_types::{DeviceId, TenantId};

    fn ticket(class: TrafficClass) -> AdmissionTicket {
        AdmissionTicket {
            source_id: 1,
            class,
            device_key: DeviceKey::new(TenantId::generate(), DeviceId::generate()),
        }
    }

    fn config() -> AdmissionPolicyConfig {
        AdmissionPolicyConfig {
            rate_capacity_tokens: 2,
            rate_refill_tokens_per_sec: 1,
            rate_max_keys: 16,
            coalesce_max_tracked: 16,
            dead_letter_capacity: 16,
            backlog_high_watermark: 10,
            backlog_low_watermark: 2,
        }
    }

    #[test]
    fn rate_limit_after_burst() {
        let policy = AdmissionPolicy::new(&config());
        let t = ticket(TrafficClass::Command);
        assert_eq!(policy.pre_admit(0, 0, t).action, PreAdmitAction::Admit);
        assert_eq!(policy.pre_admit(0, 0, t).action, PreAdmitAction::Admit);
        assert_eq!(policy.pre_admit(0, 0, t).action, PreAdmitAction::RateLimit);
    }

    #[test]
    fn coalesces_keepalive_until_released() {
        let policy = AdmissionPolicy::new(&AdmissionPolicyConfig {
            rate_capacity_tokens: 100,
            rate_refill_tokens_per_sec: 100,
            ..config()
        });
        let t = ticket(TrafficClass::Keepalive);
        assert_eq!(policy.pre_admit(0, 0, t).action, PreAdmitAction::Admit);
        assert_eq!(policy.pre_admit(0, 0, t).action, PreAdmitAction::Coalesce);
        policy.release_coalescible(t.device_key, t.class);
        assert_eq!(policy.pre_admit(0, 0, t).action, PreAdmitAction::Admit);
    }

    #[test]
    fn sheds_low_priority_when_overloaded() {
        let policy = AdmissionPolicy::new(&config());
        let keepalive = ticket(TrafficClass::Keepalive);
        let command = ticket(TrafficClass::Command);
        // Depth above high watermark enters overload.
        let decision = policy.pre_admit(0, 20, keepalive);
        assert_eq!(decision.action, PreAdmitAction::Shed);
        assert!(decision.entered_overload);
        // High priority commands are still admitted under overload.
        assert_eq!(
            policy.pre_admit(0, 20, command).action,
            PreAdmitAction::Admit
        );
    }

    #[test]
    fn redrive_only_after_recovery() {
        let policy = AdmissionPolicy::new(&config());
        let t = ticket(TrafficClass::Keepalive);
        // Enter overload and shed + dead-letter.
        assert_eq!(policy.pre_admit(0, 20, t).action, PreAdmitAction::Shed);
        policy.dead_letter(
            t,
            RuntimeMessage::ProtocolEvent {
                device_key: t.device_key,
                payload: vec![1],
            },
            DeadLetterReason::PriorityShed,
            0,
        );
        // Still overloaded: no redrive.
        assert!(policy.take_redrive_batch(10).is_none());
        // Recover below low watermark.
        assert!(
            policy.pre_admit(0, 1, ticket(TrafficClass::Command)).action == PreAdmitAction::Admit
        );
        let batch = policy.take_redrive_batch(10);
        assert_eq!(batch.map(|b| b.len()), Some(1));
    }

    #[test]
    fn dead_letter_releases_coalescer_key() {
        let policy = AdmissionPolicy::new(&AdmissionPolicyConfig {
            rate_capacity_tokens: 100,
            rate_refill_tokens_per_sec: 100,
            ..config()
        });
        let t = ticket(TrafficClass::Keepalive);
        // Admit path observes the coalescer before the message is enqueued.
        assert_eq!(policy.pre_admit(0, 0, t).action, PreAdmitAction::Admit);
        // If the message is dead-lettered (e.g. mailbox full), the pending key
        // must be released so the next event is admitted, not coalesced away.
        policy.dead_letter(
            t,
            RuntimeMessage::ProtocolEvent {
                device_key: t.device_key,
                payload: vec![1],
            },
            DeadLetterReason::Overloaded,
            0,
        );
        assert_eq!(policy.pre_admit(0, 0, t).action, PreAdmitAction::Admit);
    }
}

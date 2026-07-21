//! Application-layer tenant ingress admission control.
//!
//! Composes the bounded admission primitives from `cheetah-signal-types` to
//! enforce per-tenant and per-(tenant, method) rate limiting, coalescing of
//! redundant keepalive/position events and a bounded dead-letter queue with
//! redrive at the signaling application boundary — before inbound protocol
//! events are turned into `Operation`s, commands or repository writes.
//!
//! This is a reusable, dependency-injected service: it owns no I/O and all of
//! its state is bounded by [`IngressAdmissionConfig`]. Transport/protocol
//! adapters call [`TenantIngressAdmission::admit`] to decide whether an event
//! is processed, coalesced or rejected, and [`TenantIngressAdmission::release`]
//! once a coalescible event has been fully processed.

use std::sync::Mutex;

use cheetah_signal_types::admission::{
    CoalesceDecision, Coalescer, DeadLetterEntry, DeadLetterQueue, DeadLetterReason,
    KeyedRateLimiter, TokenBucketConfig, TrafficClass,
};
use cheetah_signal_types::{DeviceId, TenantId};

/// Bounded configuration for [`TenantIngressAdmission`].
#[derive(Clone, Copy, Debug)]
pub struct IngressAdmissionConfig {
    /// Burst capacity, in events, of each per-(tenant, class) rate bucket.
    pub rate_capacity_tokens: u64,
    /// Sustained per-(tenant, class) rate in events per second.
    pub rate_refill_tokens_per_sec: u64,
    /// Maximum number of distinct (tenant, class) rate buckets retained.
    pub rate_max_keys: usize,
    /// Maximum number of (tenant, device, class) keys tracked for coalescing.
    pub coalesce_max_tracked: usize,
    /// Maximum number of dead-lettered events retained for redrive.
    pub dead_letter_capacity: usize,
}

impl Default for IngressAdmissionConfig {
    fn default() -> Self {
        Self {
            rate_capacity_tokens: 256,
            rate_refill_tokens_per_sec: 128,
            rate_max_keys: 65_536,
            coalesce_max_tracked: 65_536,
            dead_letter_capacity: 8_192,
        }
    }
}

/// Decision returned by [`TenantIngressAdmission::admit`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressAdmission {
    /// The event is admitted and should be processed.
    Admit,
    /// The event is a redundant coalescible event and should be dropped; an
    /// equivalent event is already pending.
    Coalesced,
    /// The event exceeded the tenant/method rate limit and should be rejected.
    RateLimited,
}

struct State<P> {
    rate_limiter: KeyedRateLimiter<(TenantId, TrafficClass)>,
    coalescer: Coalescer<(TenantId, DeviceId, TrafficClass)>,
    dead_letter: DeadLetterQueue<P>,
    coalesced_total: u64,
    rate_limited_total: u64,
}

/// Per-tenant ingress admission controller.
///
/// `P` is the dead-letter payload type retained for redrive (for example a
/// decoded ingress event or its correlation identifiers).
pub struct TenantIngressAdmission<P> {
    state: Mutex<State<P>>,
}

impl<P> std::fmt::Debug for TenantIngressAdmission<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.lock();
        f.debug_struct("TenantIngressAdmission")
            .field("rate_keys", &state.rate_limiter.tracked_keys())
            .field("coalescer_pending", &state.coalescer.pending_len())
            .field("dead_letter_len", &state.dead_letter.len())
            .finish()
    }
}

impl<P> TenantIngressAdmission<P> {
    /// Builds a controller from configuration.
    pub fn new(config: IngressAdmissionConfig) -> Self {
        let rate_config = TokenBucketConfig {
            capacity_tokens: config.rate_capacity_tokens,
            refill_tokens_per_sec: config.rate_refill_tokens_per_sec,
        };
        let state = State {
            rate_limiter: KeyedRateLimiter::new(rate_config, config.rate_max_keys),
            coalescer: Coalescer::new(config.coalesce_max_tracked),
            dead_letter: DeadLetterQueue::new(config.dead_letter_capacity),
            coalesced_total: 0,
            rate_limited_total: 0,
        };
        Self {
            state: Mutex::new(state),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<P>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Decides whether an inbound event for `tenant_id`/`device_id` of the given
    /// traffic `class` is admitted, coalesced or rate-limited. `now_ms` is a
    /// monotonic timestamp in milliseconds.
    pub fn admit(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        class: TrafficClass,
        now_ms: i64,
    ) -> IngressAdmission {
        let mut state = self.lock();
        if !state.rate_limiter.try_acquire((tenant_id, class), now_ms) {
            state.rate_limited_total = state.rate_limited_total.saturating_add(1);
            return IngressAdmission::RateLimited;
        }
        if class.is_coalescible() {
            match state.coalescer.observe((tenant_id, device_id, class)) {
                CoalesceDecision::Coalesced => {
                    state.coalesced_total = state.coalesced_total.saturating_add(1);
                    return IngressAdmission::Coalesced;
                }
                CoalesceDecision::Admit => {}
            }
        }
        IngressAdmission::Admit
    }

    /// Releases a coalescible pending event once it has been fully processed so
    /// a subsequent equivalent event is admitted rather than coalesced.
    pub fn release(&self, tenant_id: TenantId, device_id: DeviceId, class: TrafficClass) {
        if class.is_coalescible() {
            self.lock()
                .coalescer
                .release(&(tenant_id, device_id, class));
        }
    }

    /// Records a dead-lettered payload for later redrive.
    pub fn dead_letter(&self, payload: P, reason: DeadLetterReason, now_ms: i64) {
        self.lock().dead_letter.push(payload, reason, now_ms);
    }

    /// Drains up to `max` dead-lettered payloads for redrive.
    pub fn drain_dead_letter(&self, max: usize) -> Vec<DeadLetterEntry<P>> {
        self.lock().dead_letter.drain(max)
    }

    /// Returns the current number of dead-lettered payloads awaiting redrive.
    pub fn dead_letter_len(&self) -> usize {
        self.lock().dead_letter.len()
    }

    /// Returns the cumulative number of coalesced events.
    pub fn coalesced_total(&self) -> u64 {
        self.lock().coalesced_total
    }

    /// Returns the cumulative number of rate-limited events.
    pub fn rate_limited_total(&self) -> u64 {
        self.lock().rate_limited_total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admission() -> TenantIngressAdmission<u32> {
        TenantIngressAdmission::new(IngressAdmissionConfig {
            rate_capacity_tokens: 2,
            rate_refill_tokens_per_sec: 1,
            rate_max_keys: 16,
            coalesce_max_tracked: 16,
            dead_letter_capacity: 8,
        })
    }

    #[test]
    fn rate_limits_per_tenant_and_method() {
        let a = admission();
        let tenant = TenantId::generate();
        let device = DeviceId::generate();
        assert_eq!(
            a.admit(tenant, device, TrafficClass::Catalog, 0),
            IngressAdmission::Admit
        );
        assert_eq!(
            a.admit(tenant, device, TrafficClass::Catalog, 0),
            IngressAdmission::Admit
        );
        assert_eq!(
            a.admit(tenant, device, TrafficClass::Catalog, 0),
            IngressAdmission::RateLimited
        );
        // A different method for the same tenant has an independent budget.
        assert_eq!(
            a.admit(tenant, device, TrafficClass::Command, 0),
            IngressAdmission::Admit
        );
        assert_eq!(a.rate_limited_total(), 1);
    }

    #[test]
    fn coalesces_keepalive_until_released() {
        let a = TenantIngressAdmission::<u32>::new(IngressAdmissionConfig {
            rate_capacity_tokens: 100,
            rate_refill_tokens_per_sec: 100,
            rate_max_keys: 16,
            coalesce_max_tracked: 16,
            dead_letter_capacity: 8,
        });
        let tenant = TenantId::generate();
        let device = DeviceId::generate();
        assert_eq!(
            a.admit(tenant, device, TrafficClass::Keepalive, 0),
            IngressAdmission::Admit
        );
        assert_eq!(
            a.admit(tenant, device, TrafficClass::Keepalive, 0),
            IngressAdmission::Coalesced
        );
        a.release(tenant, device, TrafficClass::Keepalive);
        assert_eq!(
            a.admit(tenant, device, TrafficClass::Keepalive, 0),
            IngressAdmission::Admit
        );
        assert_eq!(a.coalesced_total(), 1);
    }

    #[test]
    fn dead_letter_is_bounded_and_drainable() {
        let a = admission();
        for i in 0..10u32 {
            a.dead_letter(i, DeadLetterReason::RateLimited, 0);
        }
        // Capacity is 8; the two oldest are dropped.
        assert_eq!(a.dead_letter_len(), 8);
        let drained = a.drain_dead_letter(100);
        assert_eq!(drained.len(), 8);
        assert_eq!(a.dead_letter_len(), 0);
    }
}

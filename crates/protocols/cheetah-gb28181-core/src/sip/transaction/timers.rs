//! RFC 3261 transaction timer constants and configuration.

use std::time::Duration;

/// Transport reliability classification for transaction timers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportKind {
    /// UDP: timers A, B, D, E, F, G, H, I, J, K are used.
    Unreliable,
    /// TCP/SCTP: no application-layer retransmission; absorption timers are zero.
    Reliable,
}

impl TransportKind {
    /// True for reliable transports.
    pub fn is_reliable(self) -> bool {
        matches!(self, Self::Reliable)
    }
}

/// RFC 3261 transaction timer identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TimerKind {
    /// INVITE request retransmission (client).
    A,
    /// INVITE transaction timeout (client).
    B,
    /// Non-INVITE transaction timeout (client).
    F,
    /// Wait for ACK after non-2xx final response (INVITE client).
    D,
    /// Non-INVITE request retransmission (client).
    E,
    /// INVITE 1xx retransmission (server).
    G,
    /// INVITE final response retransmission (server).
    H,
    /// Wait in Confirmed state (server INVITE).
    I,
    /// Wait in Completed state (server non-INVITE / client non-INVITE).
    J,
    /// Wait in Completed state (client non-INVITE).
    K,
}

/// Defaults per RFC 3261 unless otherwise configured.
const DEFAULT_T1_MS: u64 = 500;
const DEFAULT_T2_MS: u64 = 4_000;
const DEFAULT_T4_MS: u64 = 5_000;
const RETRY_CAP_FACTOR: u64 = 64;

/// Timer configuration and retransmission schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionConfig {
    /// Estimated round-trip time.
    pub t1: Duration,
    /// Maximum retransmission interval for non-INVITE requests and 1xx responses.
    pub t2: Duration,
    /// Maximum duration a message can remain in the network.
    pub t4: Duration,
    /// Transport used for this transaction.
    pub transport: TransportKind,
}

impl Default for TransactionConfig {
    fn default() -> Self {
        Self {
            t1: Duration::from_millis(DEFAULT_T1_MS),
            t2: Duration::from_millis(DEFAULT_T2_MS),
            t4: Duration::from_millis(DEFAULT_T4_MS),
            transport: TransportKind::Unreliable,
        }
    }
}

impl TransactionConfig {
    /// Creates a config for the given transport.
    pub fn for_transport(transport: TransportKind) -> Self {
        Self {
            transport,
            ..Self::default()
        }
    }

    /// Timer A interval for the nth INVITE retransmission (0-indexed), capped at T2.
    pub fn timer_a(&self, n: u32) -> Duration {
        if self.transport.is_reliable() {
            return Duration::ZERO;
        }
        let factor = 1u64.saturating_mul(2u64.saturating_pow(n));
        let t1_ms = millis_u64(self.t1);
        let t2_ms = millis_u64(self.t2);
        let cap = t2_ms / std::cmp::max(t1_ms, 1);
        let capped = std::cmp::min(factor, cap);
        let capped = std::cmp::max(capped, 1);
        scale_duration(self.t1, capped)
    }

    /// Timer B (INVITE timeout) = 64 * T1.
    pub fn timer_b(&self) -> Duration {
        self.t1 * RETRY_CAP_FACTOR as u32
    }

    /// Timer C is a proxy timer; not used at the UAC/UAS core.
    pub fn timer_c(&self) -> Duration {
        Duration::from_secs(180)
    }

    /// Timer D (wait for ACK after non-2xx INVITE response) = 32s for UDP, 0 for reliable.
    pub fn timer_d(&self) -> Duration {
        if self.transport.is_reliable() {
            Duration::ZERO
        } else {
            Duration::from_secs(32)
        }
    }

    /// Timer E interval for the nth non-INVITE retransmission (0-indexed), capped at T2.
    pub fn timer_e(&self, n: u32) -> Duration {
        if self.transport.is_reliable() {
            return Duration::ZERO;
        }
        let factor = 1u64.saturating_mul(2u64.saturating_pow(n));
        let t1_ms = millis_u64(self.t1);
        let t2_ms = millis_u64(self.t2);
        let cap = t2_ms / std::cmp::max(t1_ms, 1);
        let capped = std::cmp::min(factor, cap);
        let capped = std::cmp::max(capped, 1);
        scale_duration(self.t1, capped)
    }

    /// Timer F (non-INVITE timeout) = 64 * T1.
    pub fn timer_f(&self) -> Duration {
        self.t1 * RETRY_CAP_FACTOR as u32
    }

    /// Timer G (1xx retransmission) starts at T1 and doubles up to T2.
    pub fn timer_g(&self, n: u32) -> Duration {
        if self.transport.is_reliable() {
            return Duration::ZERO;
        }
        self.timer_e(n)
    }

    /// Timer H (INVITE final response retransmission) = 64 * T1.
    pub fn timer_h(&self) -> Duration {
        self.t1 * RETRY_CAP_FACTOR as u32
    }

    /// Timer I (Confirmed state for INVITE server) = T4 for UDP, 0 for reliable.
    pub fn timer_i(&self) -> Duration {
        if self.transport.is_reliable() {
            Duration::ZERO
        } else {
            self.t4
        }
    }

    /// Timer J (Completed state for non-INVITE server) = 64 * T1 for UDP, 0 for reliable.
    pub fn timer_j(&self) -> Duration {
        if self.transport.is_reliable() {
            Duration::ZERO
        } else {
            self.timer_b()
        }
    }

    /// Timer K (Completed state for non-INVITE client) = T4 for UDP, 0 for reliable.
    pub fn timer_k(&self) -> Duration {
        if self.transport.is_reliable() {
            Duration::ZERO
        } else {
            self.t4
        }
    }
}

/// Helper to manage a set of active timers by kind.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TimerSet {
    kinds: Vec<TimerKind>,
}

impl TimerSet {
    /// Creates an empty timer set.
    pub fn new() -> Self {
        Self { kinds: Vec::new() }
    }

    /// Schedules a timer kind, replacing any previous entry.
    pub fn set(&mut self, kind: TimerKind) {
        if !self.kinds.contains(&kind) {
            self.kinds.push(kind);
        }
    }

    /// Cancels a timer kind.
    pub fn cancel(&mut self, kind: TimerKind) {
        self.kinds.retain(|k| *k != kind);
    }

    /// Returns true if the timer kind is currently scheduled.
    pub fn contains(&self, kind: TimerKind) -> bool {
        self.kinds.contains(&kind)
    }

    /// Drains and returns the scheduled kinds.
    pub fn take_all(&mut self) -> Vec<TimerKind> {
        std::mem::take(&mut self.kinds)
    }
}

/// Returns the duration in whole milliseconds, clamped to `u64::MAX`.
fn millis_u64(d: Duration) -> u64 {
    d.as_millis().min(u64::MAX as u128) as u64
}

/// Multiplies `d` by `multiplier` without overflowing the `u32` `Mul` bound on `Duration`.
fn scale_duration(d: Duration, multiplier: u64) -> Duration {
    let nanos = d
        .as_nanos()
        .saturating_mul(multiplier as u128)
        .min(u64::MAX as u128);
    Duration::from_nanos(nanos as u64)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn timer_a_doubles_and_caps() {
        let cfg = TransactionConfig::default();
        assert_eq!(cfg.timer_a(0), Duration::from_millis(500));
        assert_eq!(cfg.timer_a(1), Duration::from_millis(1_000));
        assert_eq!(cfg.timer_a(2), Duration::from_millis(2_000));
        assert_eq!(cfg.timer_a(3), Duration::from_millis(4_000));
        assert_eq!(cfg.timer_a(4), Duration::from_millis(4_000));
    }

    #[test]
    fn reliable_timers_are_zero() {
        let cfg = TransactionConfig::for_transport(TransportKind::Reliable);
        assert_eq!(cfg.timer_a(0), Duration::ZERO);
        assert_eq!(cfg.timer_d(), Duration::ZERO);
        assert_eq!(cfg.timer_e(0), Duration::ZERO);
        assert_eq!(cfg.timer_g(0), Duration::ZERO);
        assert_eq!(cfg.timer_i(), Duration::ZERO);
        assert_eq!(cfg.timer_j(), Duration::ZERO);
        assert_eq!(cfg.timer_k(), Duration::ZERO);
    }

    #[test]
    fn timer_a_does_not_panic_with_zero_t1() {
        let cfg = TransactionConfig {
            t1: Duration::ZERO,
            ..TransactionConfig::default()
        };
        let _ = cfg.timer_a(0);
    }

    #[test]
    fn timer_a_does_not_panic_with_huge_t1_and_large_n() {
        let cfg = TransactionConfig {
            t1: Duration::from_secs(u64::MAX),
            t2: Duration::from_secs(u64::MAX),
            ..TransactionConfig::default()
        };
        let value = cfg.timer_a(u32::MAX);
        assert!(value > Duration::ZERO, "huge timers must remain positive");
    }
}

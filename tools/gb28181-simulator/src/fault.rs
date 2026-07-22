//! Deterministic transport fault engine.
//!
//! The fault engine transforms a single outbound [`Frame`] into zero or more
//! timed deliveries, applying the scenario's fault rules in declaration order.
//! Each rule owns an independent RNG stream so that toggling one rule does not
//! perturb another rule's draws, keeping runs reproducible.

use crate::rng::{bernoulli, indexed_rng};
use crate::scenario::{Direction, FaultKind, MessageClass, Scenario, Transport};
use crate::wire::Frame;
use rand::Rng;
use rand::rngs::StdRng;
use serde::Serialize;

/// One timed delivery emitted by the fault engine.
#[derive(Clone, Debug)]
pub struct Delivery {
    /// Extra latency (ms) to add to the base one-way latency.
    pub relative_ms: u64,
    /// Bytes to deliver (the whole frame, possibly corrupted).
    pub bytes: Vec<u8>,
    /// When true (TCP only), the receiver ingests the bytes in two chunks,
    /// exercising incremental stream reassembly (half-packet framing).
    pub split: bool,
}

/// Aggregate fault counters recorded in the run report.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct FaultCounts {
    /// Frames dropped before delivery.
    pub dropped: u64,
    /// Frames delivered with injected extra latency.
    pub delayed: u64,
    /// Frames reordered past later frames.
    pub reordered: u64,
    /// Extra duplicate copies delivered.
    pub duplicated: u64,
    /// Frames split into partial stream fragments.
    pub half_split: u64,
    /// Frames corrupted so parsing fails.
    pub malformed: u64,
    /// SIP error responses injected by the platform.
    pub sip_errors_injected: u64,
}

/// Base one-way latency applied to every delivery, in milliseconds.
pub const BASE_LATENCY_MS: u64 = 5;

/// A compiled fault rule with its own RNG stream.
#[derive(Debug)]
struct TransportRule {
    kind: FaultKind,
    rng: StdRng,
}

/// Deterministic fault engine over the scenario's transport faults.
#[derive(Debug)]
pub struct FaultEngine {
    transport: Transport,
    rules: Vec<TransportRule>,
    sip_errors: Vec<TransportRule>,
    counts: FaultCounts,
}

impl FaultEngine {
    /// Builds a fault engine from a scenario, deriving one RNG per rule.
    pub fn new(scenario: &Scenario) -> Self {
        let mut rules = Vec::new();
        let mut sip_errors = Vec::new();
        for (index, kind) in scenario.faults.iter().enumerate() {
            let rng = indexed_rng(scenario.seed, "fault", index as u64);
            let rule = TransportRule {
                kind: kind.clone(),
                rng,
            };
            match kind {
                FaultKind::SipError { .. } => sip_errors.push(rule),
                _ => rules.push(rule),
            }
        }
        Self {
            transport: scenario.transport,
            rules,
            sip_errors,
            counts: FaultCounts::default(),
        }
    }

    /// Returns the accumulated fault counters.
    pub fn counts(&self) -> &FaultCounts {
        &self.counts
    }

    /// Applies all transport faults to `frame`, returning timed deliveries.
    ///
    /// An empty result means the frame was dropped.
    pub fn apply(&mut self, frame: &Frame) -> Vec<Delivery> {
        let from_device = frame.from_device();
        let mut bytes = frame.bytes.clone();
        let mut extra_latency_ms = 0u64;
        let mut duplicate = false;
        let mut split = false;
        let mut corrupt = false;

        for rule in &mut self.rules {
            match &rule.kind {
                FaultKind::Drop {
                    rate,
                    direction,
                    target,
                } => {
                    if selects(*direction, *target, from_device, frame.class)
                        && bernoulli(&mut rule.rng, *rate)
                    {
                        self.counts.dropped += 1;
                        return Vec::new();
                    }
                }
                FaultKind::Delay {
                    extra_ms,
                    jitter_ms,
                    rate,
                    direction,
                    target,
                } => {
                    if selects(*direction, *target, from_device, frame.class)
                        && bernoulli(&mut rule.rng, *rate)
                    {
                        let jitter = if *jitter_ms > 0 {
                            rule.rng.gen_range(0..=*jitter_ms)
                        } else {
                            0
                        };
                        self.counts.delayed += 1;
                        extra_ms_accumulate(&mut extra_latency_ms, *extra_ms + jitter);
                    }
                }
                FaultKind::Reorder {
                    window,
                    rate,
                    direction,
                } => {
                    if direction.matches(from_device) && bernoulli(&mut rule.rng, *rate) {
                        self.counts.reordered += 1;
                        extra_ms_accumulate(
                            &mut extra_latency_ms,
                            u64::from(*window) * (BASE_LATENCY_MS * 4),
                        );
                    }
                }
                FaultKind::Duplicate {
                    rate,
                    direction,
                    target,
                } => {
                    if selects(*direction, *target, from_device, frame.class)
                        && bernoulli(&mut rule.rng, *rate)
                    {
                        duplicate = true;
                    }
                }
                FaultKind::HalfPacket { rate, direction } => {
                    if self.transport == Transport::Tcp
                        && direction.matches(from_device)
                        && bernoulli(&mut rule.rng, *rate)
                        && bytes.len() >= 2
                    {
                        split = true;
                    }
                }
                FaultKind::Malformed {
                    rate,
                    direction,
                    target,
                } => {
                    if selects(*direction, *target, from_device, frame.class)
                        && bernoulli(&mut rule.rng, *rate)
                        && !bytes.is_empty()
                    {
                        corrupt = true;
                    }
                }
                FaultKind::SipError { .. } => {}
            }
        }

        if corrupt {
            self.counts.malformed += 1;
            // Zero the first byte so both request start lines and response
            // status lines fail to parse deterministically.
            bytes[0] = 0;
        }

        if split {
            self.counts.half_split += 1;
        }

        let mut deliveries = vec![Delivery {
            relative_ms: extra_latency_ms,
            bytes: bytes.clone(),
            split,
        }];

        if duplicate {
            self.counts.duplicated += 1;
            deliveries.push(Delivery {
                relative_ms: extra_latency_ms + BASE_LATENCY_MS,
                bytes,
                split: false,
            });
        }

        deliveries
    }

    /// Returns an injected SIP error status for a device request of `class`,
    /// or `None` when no matching error rule fires.
    pub fn sip_error_for(&mut self, class: MessageClass) -> Option<u16> {
        for rule in &mut self.sip_errors {
            if let FaultKind::SipError { rate, code, target } = &rule.kind
                && target.matches(class)
                && bernoulli(&mut rule.rng, *rate)
            {
                self.counts.sip_errors_injected += 1;
                return Some(*code);
            }
        }
        None
    }
}

fn selects(
    direction: Direction,
    target: MessageClass,
    from_device: bool,
    class: MessageClass,
) -> bool {
    direction.matches(from_device) && target.matches(class)
}

fn extra_ms_accumulate(total: &mut u64, add: u64) {
    *total = total.saturating_add(add);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::wire::Endpoint;

    fn frame(class: MessageClass, from_device: bool, len: usize) -> Frame {
        Frame {
            from: if from_device {
                Endpoint::Device(0)
            } else {
                Endpoint::Platform
            },
            to: if from_device {
                Endpoint::Platform
            } else {
                Endpoint::Device(0)
            },
            bytes: vec![b'A'; len],
            class,
            summary: "test".to_string(),
        }
    }

    fn scenario_with(faults: Vec<FaultKind>, transport: Transport) -> Scenario {
        Scenario {
            faults,
            transport,
            ..Scenario::default()
        }
    }

    #[test]
    fn certain_drop_removes_frame() {
        let scenario = scenario_with(
            vec![FaultKind::Drop {
                rate: 1.0,
                direction: Direction::Both,
                target: MessageClass::Any,
            }],
            Transport::Udp,
        );
        let mut engine = FaultEngine::new(&scenario);
        let deliveries = engine.apply(&frame(MessageClass::Register, true, 10));
        assert!(deliveries.is_empty());
        assert_eq!(engine.counts().dropped, 1);
    }

    #[test]
    fn drop_respects_target_class() {
        let scenario = scenario_with(
            vec![FaultKind::Drop {
                rate: 1.0,
                direction: Direction::Both,
                target: MessageClass::Keepalive,
            }],
            Transport::Udp,
        );
        let mut engine = FaultEngine::new(&scenario);
        // Register frame is not targeted, so it survives.
        assert_eq!(
            engine.apply(&frame(MessageClass::Register, true, 10)).len(),
            1
        );
        assert!(
            engine
                .apply(&frame(MessageClass::Keepalive, true, 10))
                .is_empty()
        );
    }

    #[test]
    fn half_packet_splits_only_on_tcp() {
        let scenario = scenario_with(
            vec![FaultKind::HalfPacket {
                rate: 1.0,
                direction: Direction::Both,
            }],
            Transport::Tcp,
        );
        let mut engine = FaultEngine::new(&scenario);
        let deliveries = engine.apply(&frame(MessageClass::Register, true, 10));
        assert_eq!(deliveries.len(), 1);
        assert!(deliveries[0].split);
        assert_eq!(engine.counts().half_split, 1);
    }

    #[test]
    fn duplicate_emits_extra_copy() {
        let scenario = scenario_with(
            vec![FaultKind::Duplicate {
                rate: 1.0,
                direction: Direction::Both,
                target: MessageClass::Any,
            }],
            Transport::Udp,
        );
        let mut engine = FaultEngine::new(&scenario);
        let deliveries = engine.apply(&frame(MessageClass::Register, true, 10));
        assert_eq!(deliveries.len(), 2);
        assert_eq!(engine.counts().duplicated, 1);
    }

    #[test]
    fn malformed_corrupts_first_byte() {
        let scenario = scenario_with(
            vec![FaultKind::Malformed {
                rate: 1.0,
                direction: Direction::Both,
                target: MessageClass::Any,
            }],
            Transport::Udp,
        );
        let mut engine = FaultEngine::new(&scenario);
        let deliveries = engine.apply(&frame(MessageClass::Register, true, 10));
        assert_eq!(deliveries[0].bytes[0], 0);
        assert_eq!(engine.counts().malformed, 1);
    }

    #[test]
    fn sip_error_matches_target() {
        let scenario = scenario_with(
            vec![FaultKind::SipError {
                rate: 1.0,
                code: 503,
                target: MessageClass::Register,
            }],
            Transport::Udp,
        );
        let mut engine = FaultEngine::new(&scenario);
        assert_eq!(engine.sip_error_for(MessageClass::Register), Some(503));
        assert_eq!(engine.sip_error_for(MessageClass::Keepalive), None);
    }
}

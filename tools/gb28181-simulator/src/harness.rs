//! Fixed-shard, discrete-event simulator harness.
//!
//! The harness owns a single timer wheel and a fixed number of shard workers
//! that manage many lazy device states.  There is no per-device task or timer:
//! device start and keepalive are staggered by a seeded RNG and scheduled onto
//! the shared wheel.  A run is fully deterministic and produces a [`RunReport`].

use crate::clock::TimerWheel;
use crate::device::{Device, DeviceEvent, DeviceInput};
use crate::fault::{BASE_LATENCY_MS, FaultEngine};
use crate::platform::{Platform, PlatformEvent};
use crate::profile::ResolvedProfile;
use crate::report::{MessageCounts, Outcomes, Resources, RunReport};
use crate::rng::indexed_rng;
use crate::scenario::{Scenario, StepKind};
use crate::transport::Assembler;
use crate::wire::{Endpoint, Frame, classify, summarize};
use cheetah_gb28181_core::{SipMessage, encode_message};
use rand::Rng;
use secrecy::SecretString;
use sha2::{Digest, Sha256};

/// A scheduled simulator event.
#[derive(Debug)]
enum Event {
    /// Begin registration for a device.
    DeviceStart(u32),
    /// Keepalive interval elapsed for a device.
    DeviceKeepalive(u32),
    /// Deliver bytes on an endpoint edge.
    Deliver {
        from: Endpoint,
        to: Endpoint,
        bytes: Vec<u8>,
        split: bool,
    },
    /// A scripted platform step fires.
    Step(usize),
}

/// The simulator harness.
#[derive(Debug)]
pub struct Harness {
    scenario: Scenario,
    profile: ResolvedProfile,
    wheel: TimerWheel<Event>,
    devices: Vec<Device>,
    platform: Platform,
    fault: FaultEngine,
    assembler: Assembler,
    counts: MessageCounts,
    outcomes: Outcomes,
    transcript: Sha256,
    transcript_entries: u64,
}

impl Harness {
    /// Builds a harness from a validated scenario.
    pub fn new(scenario: Scenario) -> Self {
        let profile = ResolvedProfile::resolve(&scenario.profile);
        let domain = "platform".to_string();
        let server = "platform:5060".to_string();
        let mut devices = Vec::with_capacity(scenario.device_count as usize);
        for index in 0..scenario.device_count {
            let device_id = device_id(&scenario.base_device_id, index);
            devices.push(Device::new(
                index,
                device_id,
                SecretString::new("12345678".to_string().into()),
                profile.clone(),
                server.clone(),
                domain.clone(),
            ));
        }
        let fault = FaultEngine::new(&scenario);
        let assembler = Assembler::new(scenario.transport);
        Self {
            profile,
            devices,
            platform: Platform::new(domain),
            fault,
            assembler,
            counts: MessageCounts::default(),
            outcomes: Outcomes::default(),
            transcript: Sha256::new(),
            transcript_entries: 0,
            wheel: TimerWheel::new(),
            scenario,
        }
    }

    /// Runs the scenario to completion and returns the reproducible report.
    pub fn run(mut self) -> RunReport {
        self.seed_schedule();
        while let Some((now, event)) = self.wheel.pop() {
            if now > self.scenario.duration_ms {
                break;
            }
            self.process(now, event);
        }
        self.finish()
    }

    fn seed_schedule(&mut self) {
        for index in 0..self.scenario.device_count {
            let start = self.stagger_offset(index);
            self.wheel.schedule(start, Event::DeviceStart(index));
        }
        for (idx, step) in self.scenario.steps.iter().enumerate() {
            self.wheel.schedule(step.at_ms(), Event::Step(idx));
        }
    }

    fn stagger_offset(&self, index: u32) -> u64 {
        if self.scenario.register_stagger_ms == 0 {
            return 0;
        }
        let shard = index % self.scenario.shards;
        let mut rng = indexed_rng(
            self.scenario.seed,
            "stagger",
            u64::from(index) ^ (u64::from(shard) << 32),
        );
        rng.gen_range(0..self.scenario.register_stagger_ms)
    }

    fn process(&mut self, now: u64, event: Event) {
        match event {
            Event::DeviceStart(index) => {
                let step = self.devices[index as usize].step(DeviceInput::Start);
                self.dispatch_device_step(index, step);
                let next = now
                    .saturating_add(self.profile.keepalive_ms)
                    .saturating_add(self.stagger_offset(index) % self.profile.keepalive_ms.max(1));
                self.wheel.schedule(next, Event::DeviceKeepalive(index));
            }
            Event::DeviceKeepalive(index) => {
                let step = self.devices[index as usize].step(DeviceInput::KeepaliveTick);
                self.dispatch_device_step(index, step);
                self.wheel
                    .schedule_after(self.profile.keepalive_ms, Event::DeviceKeepalive(index));
            }
            Event::Step(idx) => self.run_step(idx),
            Event::Deliver {
                from,
                to,
                bytes,
                split,
            } => self.deliver(from, to, &bytes, split),
        }
    }

    fn run_step(&mut self, idx: usize) {
        let Some(step) = self.scenario.steps.get(idx).cloned() else {
            return;
        };
        let registered: Vec<u32> = self
            .devices
            .iter()
            .enumerate()
            .filter(|(_, d)| d.registered())
            .map(|(i, _)| i as u32)
            .collect();
        for index in registered {
            let device_id = self.devices[index as usize].device_id().to_string();
            let msg = match &step {
                StepKind::CatalogQuery { .. } => self.platform.catalog_query(&device_id),
                StepKind::Invite { .. } => self.platform.invite(&device_id),
                StepKind::Bye { .. } => self.platform.bye(&device_id),
            };
            if let Some(msg) = msg {
                self.send(Endpoint::Platform, Endpoint::Device(index), &msg);
            }
        }
    }

    fn dispatch_device_step(&mut self, index: u32, step: crate::device::DeviceStep) {
        for event in step.events {
            self.record_device_event(event);
        }
        for msg in step.messages {
            self.send(Endpoint::Device(index), Endpoint::Platform, &msg);
        }
    }

    fn deliver(&mut self, from: Endpoint, to: Endpoint, bytes: &[u8], split: bool) {
        let ingested = self.assembler.ingest(from, to, bytes, split);
        self.counts.delivered += 1;
        self.counts.parse_errors += ingested.parse_errors;
        for msg in ingested.messages {
            self.route(from, to, &msg);
        }
    }

    fn route(&mut self, from: Endpoint, to: Endpoint, msg: &SipMessage) {
        match to {
            Endpoint::Platform => {
                let sip_error = if matches!(msg, SipMessage::Request { .. }) {
                    self.fault.sip_error_for(classify(msg))
                } else {
                    None
                };
                let step = self.platform.on_inbound(msg, sip_error);
                for event in step.events {
                    self.record_platform_event(event);
                }
                for reply in step.messages {
                    self.send(Endpoint::Platform, from, &reply);
                }
            }
            Endpoint::Device(index) => {
                if let Some(device) = self.devices.get_mut(index as usize) {
                    let step = device.step(DeviceInput::Inbound(msg));
                    for event in step.events {
                        self.record_device_event(event);
                    }
                    for reply in step.messages {
                        self.send(Endpoint::Device(index), Endpoint::Platform, &reply);
                    }
                }
            }
        }
    }

    fn send(&mut self, from: Endpoint, to: Endpoint, msg: &SipMessage) {
        let bytes = encode_message(msg);
        let class = classify(msg);
        let summary = summarize(msg);
        self.counts.sent += 1;
        self.record_transcript(from, to, &summary);
        let frame = Frame {
            from,
            to,
            bytes,
            class,
            summary,
        };
        let deliveries = self.fault.apply(&frame);
        for delivery in deliveries {
            let at = self
                .wheel
                .now_ms()
                .saturating_add(BASE_LATENCY_MS)
                .saturating_add(delivery.relative_ms);
            self.wheel.schedule(
                at,
                Event::Deliver {
                    from,
                    to,
                    bytes: delivery.bytes,
                    split: delivery.split,
                },
            );
        }
    }

    fn record_transcript(&mut self, from: Endpoint, to: Endpoint, summary: &str) {
        self.transcript
            .update(self.transcript_entries.to_le_bytes());
        self.transcript.update(from.label().as_bytes());
        self.transcript.update(b">");
        self.transcript.update(to.label().as_bytes());
        self.transcript.update(b"|");
        self.transcript.update(summary.as_bytes());
        self.transcript.update(b"\n");
        self.transcript_entries += 1;
    }

    fn record_device_event(&mut self, event: DeviceEvent) {
        match event {
            DeviceEvent::Registered => self.outcomes.devices_registered += 1,
            DeviceEvent::KeepaliveSent => self.outcomes.keepalives_sent += 1,
            DeviceEvent::CatalogResponded => self.outcomes.catalog_responses += 1,
            DeviceEvent::MediaInviteAnswered => self.outcomes.invites_answered += 1,
            DeviceEvent::MediaByeAnswered => self.outcomes.byes_answered += 1,
            DeviceEvent::ErrorObserved => {}
        }
    }

    fn record_platform_event(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::Challenged => self.outcomes.challenges += 1,
            PlatformEvent::RegisterAccepted => self.outcomes.registrations_accepted += 1,
            PlatformEvent::KeepaliveAcked => self.outcomes.keepalives_acked += 1,
            PlatformEvent::CatalogReceived => self.outcomes.catalog_received += 1,
            PlatformEvent::ErrorInjected => self.outcomes.errors_injected += 1,
        }
    }

    fn finish(self) -> RunReport {
        let digest = self.transcript.finalize();
        let transcript_hash = hex_lower(&digest);
        RunReport {
            scenario_name: self.scenario.name,
            seed: self.scenario.seed,
            transport: format!("{:?}", self.scenario.transport).to_lowercase(),
            profile: self.profile.id,
            standard: self.profile.standard,
            synthetic_vendor: self.profile.synthetic_vendor,
            device_count: self.scenario.device_count,
            duration_ms: self.scenario.duration_ms,
            message_counts: self.counts,
            fault_counts: self.fault.counts().clone(),
            outcomes: self.outcomes,
            resources: Resources {
                shard_count: self.scenario.shards,
                udp_sockets: self.scenario.udp_sockets,
                tcp_pool: self.scenario.tcp_pool,
                peak_scheduled_events: self.wheel.peak_len() as u64,
                total_events_processed: self.wheel.processed(),
            },
            transcript_entries: self.transcript_entries,
            transcript_hash,
        }
    }
}

/// Runs a scenario end-to-end and returns its reproducible report.
pub fn run_scenario(scenario: Scenario) -> RunReport {
    Harness::new(scenario).run()
}

fn device_id(base: &str, index: u32) -> String {
    let base = base.trim();
    let suffix = format!("{index:04}");
    let max_base = 20usize.saturating_sub(suffix.chars().count());
    let base_chars: String = base.chars().take(max_base).collect();
    format!("{base_chars}{suffix}")
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::scenario::{Direction, FaultKind, MessageClass, Transport};

    fn base_scenario() -> Scenario {
        Scenario {
            name: "unit".to_string(),
            seed: 7,
            shards: 2,
            device_count: 4,
            duration_ms: 120_000,
            register_stagger_ms: 2_000,
            steps: vec![
                StepKind::CatalogQuery { at_ms: 40_000 },
                StepKind::Invite { at_ms: 60_000 },
                StepKind::Bye { at_ms: 80_000 },
            ],
            ..Scenario::default()
        }
    }

    #[test]
    fn clean_run_registers_all_devices() {
        let report = run_scenario(base_scenario());
        assert_eq!(report.outcomes.devices_registered, 4);
        assert_eq!(report.outcomes.registrations_accepted, 4);
        assert!(report.outcomes.keepalives_acked > 0);
        assert!(report.outcomes.catalog_received >= 1);
        assert!(report.outcomes.invites_answered >= 1);
        assert!(report.outcomes.byes_answered >= 1);
        assert_eq!(report.message_counts.parse_errors, 0);
    }

    #[test]
    fn run_is_reproducible() {
        let a = run_scenario(base_scenario());
        let b = run_scenario(base_scenario());
        assert_eq!(a.transcript_hash, b.transcript_hash);
        assert_eq!(a.message_counts, b.message_counts);
        assert_eq!(a.outcomes, b.outcomes);
    }

    #[test]
    fn seed_changes_transcript() {
        let mut other = base_scenario();
        other.seed = 99;
        // Stagger differs by seed, so the transcript ordering hash differs.
        assert_ne!(
            run_scenario(base_scenario()).transcript_hash,
            run_scenario(other).transcript_hash
        );
    }

    #[test]
    fn dropping_all_register_prevents_registration() {
        let mut scenario = base_scenario();
        scenario.faults = vec![FaultKind::Drop {
            rate: 1.0,
            direction: Direction::DeviceToPlatform,
            target: MessageClass::Register,
        }];
        let report = run_scenario(scenario);
        assert_eq!(report.outcomes.devices_registered, 0);
        assert!(report.fault_counts.dropped > 0);
    }

    #[test]
    fn sip_error_blocks_registration() {
        let mut scenario = base_scenario();
        scenario.faults = vec![FaultKind::SipError {
            rate: 1.0,
            code: 503,
            target: MessageClass::Register,
        }];
        let report = run_scenario(scenario);
        assert_eq!(report.outcomes.devices_registered, 0);
        assert!(report.fault_counts.sip_errors_injected > 0);
    }

    #[test]
    fn tcp_half_packet_still_reassembles() {
        let mut scenario = base_scenario();
        scenario.transport = Transport::Tcp;
        scenario.faults = vec![FaultKind::HalfPacket {
            rate: 1.0,
            direction: Direction::Both,
        }];
        let report = run_scenario(scenario);
        assert!(report.fault_counts.half_split > 0);
        // Reassembly keeps the parser contract intact: devices still register.
        assert_eq!(report.outcomes.devices_registered, 4);
        assert_eq!(report.message_counts.parse_errors, 0);
    }
}

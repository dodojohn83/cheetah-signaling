//! Deterministic fault-scenario DSL for the GB28181 simulator.
//!
//! A scenario is a declarative, seed-driven description of a repeatable
//! simulator run.  It fixes the number of shard workers, the device population,
//! the vendor/standard profile, the transport, scripted platform steps and a
//! list of fault-injection rules.  Scenarios are parsed from TOML so that a run
//! can be reproduced from a single file plus its seed.
//!
//! The DSL never describes real media payloads: media steps only exercise the
//! signalling handshake (INVITE/200/BYE) and produce media *control* events.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Errors produced while loading or validating a scenario.
#[derive(thiserror::Error, Debug)]
pub enum ScenarioError {
    /// The scenario file could not be read from disk.
    #[error("failed to read scenario file: {0}")]
    Io(#[from] std::io::Error),
    /// The scenario file was not valid TOML for the DSL schema.
    #[error("failed to parse scenario TOML: {0}")]
    Parse(#[from] toml::de::Error),
    /// The scenario parsed but violated a semantic constraint.
    #[error("invalid scenario: {0}")]
    Invalid(String),
}

/// Wire transport used by the simulated devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// Connectionless datagrams; a partial datagram is a parse error.
    #[default]
    Udp,
    /// Byte stream with reassembly; supports half-packet/coalesced framing.
    Tcp,
}

/// Direction a fault applies to, relative to the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Requests/responses the device sends to the platform.
    #[default]
    DeviceToPlatform,
    /// Requests/responses the platform sends to the device.
    PlatformToDevice,
    /// Both directions.
    Both,
}

impl Direction {
    /// Returns whether this direction selector matches a concrete edge.
    pub fn matches(self, from_device: bool) -> bool {
        match self {
            Direction::Both => true,
            Direction::DeviceToPlatform => from_device,
            Direction::PlatformToDevice => !from_device,
        }
    }
}

/// Coarse semantic class of a signalling message, used to target faults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageClass {
    /// Any message class.
    #[default]
    Any,
    /// REGISTER request / its response.
    Register,
    /// Keepalive MESSAGE / its 200.
    Keepalive,
    /// Catalog query or catalog response MESSAGE.
    Catalog,
    /// INVITE / 200 / ACK / BYE media control.
    Media,
    /// Any other MESSAGE (alarm, device info, ...).
    Message,
}

impl MessageClass {
    /// Returns whether this selector matches a concrete message class.
    pub fn matches(self, actual: MessageClass) -> bool {
        self == MessageClass::Any || self == actual
    }
}

/// A single fault-injection rule.
///
/// Rules are evaluated in declaration order against every frame; each rule
/// draws from a dedicated deterministic RNG stream so that adding or removing
/// an unrelated rule does not perturb the others.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FaultKind {
    /// Drop the frame entirely.
    Drop {
        /// Probability in `[0, 1]`.
        rate: f64,
        /// Direction selector.
        #[serde(default)]
        direction: Direction,
        /// Message-class selector.
        #[serde(default)]
        target: MessageClass,
    },
    /// Add extra latency (plus bounded jitter) to delivery.
    Delay {
        /// Base extra latency in milliseconds.
        extra_ms: u64,
        /// Uniform jitter added on top, in milliseconds.
        #[serde(default)]
        jitter_ms: u64,
        /// Probability the delay applies, in `[0, 1]`.
        #[serde(default = "one")]
        rate: f64,
        /// Direction selector.
        #[serde(default)]
        direction: Direction,
        /// Message-class selector.
        #[serde(default)]
        target: MessageClass,
    },
    /// Deliver the frame out of order by holding it back by up to `window`
    /// delivery slots.
    Reorder {
        /// Reorder window in delivery slots (>= 1).
        window: u32,
        /// Probability the reorder applies, in `[0, 1]`.
        #[serde(default = "one")]
        rate: f64,
        /// Direction selector.
        #[serde(default)]
        direction: Direction,
    },
    /// Deliver a byte-identical copy of the frame in addition to the original.
    Duplicate {
        /// Probability in `[0, 1]`.
        rate: f64,
        /// Direction selector.
        #[serde(default)]
        direction: Direction,
        /// Message-class selector.
        #[serde(default)]
        target: MessageClass,
    },
    /// Split the frame into two partial deliveries (TCP reassembly only).
    HalfPacket {
        /// Probability in `[0, 1]`.
        rate: f64,
        /// Direction selector.
        #[serde(default)]
        direction: Direction,
    },
    /// Corrupt one byte of the frame so parsing fails.
    Malformed {
        /// Probability in `[0, 1]`.
        rate: f64,
        /// Direction selector.
        #[serde(default)]
        direction: Direction,
        /// Message-class selector.
        #[serde(default)]
        target: MessageClass,
    },
    /// Make the platform answer a device request with a SIP error status.
    SipError {
        /// Probability in `[0, 1]`.
        rate: f64,
        /// SIP status code (>= 300).
        code: u16,
        /// Message-class selector for the request that triggers the error.
        #[serde(default)]
        target: MessageClass,
    },
}

fn one() -> f64 {
    1.0
}

/// A scripted, platform-initiated step at a fixed virtual time.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepKind {
    /// Platform sends a Catalog query MESSAGE to every registered device.
    CatalogQuery {
        /// Virtual time offset from run start, in milliseconds.
        at_ms: u64,
    },
    /// Platform sends an INVITE (media control only) to every registered device.
    Invite {
        /// Virtual time offset from run start, in milliseconds.
        at_ms: u64,
    },
    /// Platform sends a BYE for previously invited devices.
    Bye {
        /// Virtual time offset from run start, in milliseconds.
        at_ms: u64,
    },
}

impl StepKind {
    /// The virtual time (ms from run start) at which the step fires.
    pub fn at_ms(&self) -> u64 {
        match self {
            StepKind::CatalogQuery { at_ms }
            | StepKind::Invite { at_ms }
            | StepKind::Bye { at_ms } => *at_ms,
        }
    }
}

/// Vendor/standard profile.  Synthetic vendor names are behavioural fixtures
/// only and are explicitly **not** interoperability evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    /// Profile identifier (`generic`, `vendor-a`, ...).
    pub id: String,
    /// GB/T standard label recorded in the report.
    #[serde(default = "default_standard")]
    pub standard: String,
    /// Keepalive interval in milliseconds.
    #[serde(default = "default_keepalive_ms")]
    pub keepalive_ms: u64,
    /// Number of catalog channels reported per device.
    #[serde(default = "default_catalog_items")]
    pub catalog_items: u32,
    /// Whether this profile is a synthetic vendor fixture (non-interop).
    #[serde(default)]
    pub synthetic_vendor: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: "generic".to_string(),
            standard: default_standard(),
            keepalive_ms: default_keepalive_ms(),
            catalog_items: default_catalog_items(),
            synthetic_vendor: false,
        }
    }
}

fn default_standard() -> String {
    "GB/T 28181-2022".to_string()
}

fn default_keepalive_ms() -> u64 {
    30_000
}

fn default_catalog_items() -> u32 {
    1
}

/// A complete, self-contained scenario definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scenario {
    /// Human-readable scenario name (recorded in the report).
    #[serde(default = "default_name")]
    pub name: String,
    /// Master seed; all RNG streams derive from this value.
    #[serde(default)]
    pub seed: u64,
    /// Fixed number of shard workers.
    #[serde(default = "default_shards")]
    pub shards: u32,
    /// Number of simulated devices.
    #[serde(default = "default_device_count")]
    pub device_count: u32,
    /// Base device id prefix; the index is appended per device.
    #[serde(default = "default_base_device_id")]
    pub base_device_id: String,
    /// Wire transport.
    #[serde(default)]
    pub transport: Transport,
    /// Total virtual duration in milliseconds.
    #[serde(default = "default_duration_ms")]
    pub duration_ms: u64,
    /// Window over which device start/keepalive are staggered, in milliseconds.
    #[serde(default = "default_stagger_ms")]
    pub register_stagger_ms: u64,
    /// Number of UDP source sockets shared across devices (bounded resource).
    #[serde(default = "default_udp_sockets")]
    pub udp_sockets: u32,
    /// Maximum concurrent TCP connections in the pool (bounded resource).
    #[serde(default = "default_tcp_pool")]
    pub tcp_pool: u32,
    /// Vendor/standard profile.
    #[serde(default)]
    pub profile: Profile,
    /// Scripted platform steps.
    #[serde(default)]
    pub steps: Vec<StepKind>,
    /// Fault-injection rules, evaluated in order.
    #[serde(default)]
    pub faults: Vec<FaultKind>,
}

fn default_name() -> String {
    "unnamed".to_string()
}

fn default_shards() -> u32 {
    4
}

fn default_device_count() -> u32 {
    1
}

fn default_base_device_id() -> String {
    "34020000001320000001".to_string()
}

fn default_duration_ms() -> u64 {
    120_000
}

fn default_stagger_ms() -> u64 {
    5_000
}

fn default_udp_sockets() -> u32 {
    8
}

fn default_tcp_pool() -> u32 {
    64
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            name: default_name(),
            seed: 0,
            shards: default_shards(),
            device_count: default_device_count(),
            base_device_id: default_base_device_id(),
            transport: Transport::default(),
            duration_ms: default_duration_ms(),
            register_stagger_ms: default_stagger_ms(),
            udp_sockets: default_udp_sockets(),
            tcp_pool: default_tcp_pool(),
            profile: Profile::default(),
            steps: Vec::new(),
            faults: Vec::new(),
        }
    }
}

impl Scenario {
    /// Loads a scenario from a TOML file and validates it.
    ///
    /// # Errors
    ///
    /// Returns [`ScenarioError`] if the file cannot be read, parsed or fails
    /// validation.
    pub fn from_toml_path(path: impl AsRef<Path>) -> Result<Self, ScenarioError> {
        let text = std::fs::read_to_string(path)?;
        Self::from_toml_str(&text)
    }

    /// Parses and validates a scenario from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`ScenarioError::Parse`] on malformed TOML and
    /// [`ScenarioError::Invalid`] on a semantic constraint violation.
    pub fn from_toml_str(text: &str) -> Result<Self, ScenarioError> {
        let scenario: Scenario = toml::from_str(text)?;
        scenario.validate()?;
        Ok(scenario)
    }

    /// Validates all bounded fields and rejects contradictory configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ScenarioError::Invalid`] describing the first violation.
    pub fn validate(&self) -> Result<(), ScenarioError> {
        if self.shards == 0 {
            return Err(ScenarioError::Invalid("shards must be >= 1".into()));
        }
        if self.device_count == 0 {
            return Err(ScenarioError::Invalid("device_count must be >= 1".into()));
        }
        if self.duration_ms == 0 {
            return Err(ScenarioError::Invalid("duration_ms must be >= 1".into()));
        }
        if self.udp_sockets == 0 {
            return Err(ScenarioError::Invalid("udp_sockets must be >= 1".into()));
        }
        if self.tcp_pool == 0 {
            return Err(ScenarioError::Invalid("tcp_pool must be >= 1".into()));
        }
        if self.profile.keepalive_ms == 0 {
            return Err(ScenarioError::Invalid(
                "profile.keepalive_ms must be >= 1".into(),
            ));
        }
        for fault in &self.faults {
            validate_fault(fault)?;
            if self.transport == Transport::Udp && matches!(fault, FaultKind::HalfPacket { .. }) {
                return Err(ScenarioError::Invalid(
                    "half_packet fault requires transport = \"tcp\"".into(),
                ));
            }
        }
        for step in &self.steps {
            if step.at_ms() >= self.duration_ms {
                return Err(ScenarioError::Invalid(format!(
                    "step at_ms {} must be < duration_ms {}",
                    step.at_ms(),
                    self.duration_ms
                )));
            }
        }
        Ok(())
    }
}

fn check_rate(name: &str, rate: f64) -> Result<(), ScenarioError> {
    if !(0.0..=1.0).contains(&rate) {
        return Err(ScenarioError::Invalid(format!(
            "{name} rate {rate} must be within [0, 1]"
        )));
    }
    Ok(())
}

fn validate_fault(fault: &FaultKind) -> Result<(), ScenarioError> {
    match fault {
        FaultKind::Drop { rate, .. } => check_rate("drop", *rate),
        FaultKind::Delay { rate, .. } => check_rate("delay", *rate),
        FaultKind::Reorder { window, rate, .. } => {
            check_rate("reorder", *rate)?;
            if *window == 0 {
                return Err(ScenarioError::Invalid("reorder window must be >= 1".into()));
            }
            Ok(())
        }
        FaultKind::Duplicate { rate, .. } => check_rate("duplicate", *rate),
        FaultKind::HalfPacket { rate, .. } => check_rate("half_packet", *rate),
        FaultKind::Malformed { rate, .. } => check_rate("malformed", *rate),
        FaultKind::SipError { rate, code, .. } => {
            check_rate("sip_error", *rate)?;
            if *code < 300 || *code > 699 {
                return Err(ScenarioError::Invalid(format!(
                    "sip_error code {code} must be within [300, 699]"
                )));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn parses_minimal_scenario() {
        let scenario = Scenario::from_toml_str("name = \"m\"\nseed = 1\n").unwrap();
        assert_eq!(scenario.name, "m");
        assert_eq!(scenario.seed, 1);
        assert_eq!(scenario.shards, default_shards());
    }

    #[test]
    fn rejects_zero_shards() {
        let err = Scenario::from_toml_str("shards = 0\n").unwrap_err();
        assert!(matches!(err, ScenarioError::Invalid(_)));
    }

    #[test]
    fn rejects_out_of_range_rate() {
        let text = "[[faults]]\nkind = \"drop\"\nrate = 1.5\n";
        let err = Scenario::from_toml_str(text).unwrap_err();
        assert!(matches!(err, ScenarioError::Invalid(_)));
    }

    #[test]
    fn rejects_half_packet_on_udp() {
        let text = "transport = \"udp\"\n[[faults]]\nkind = \"half_packet\"\nrate = 0.1\n";
        let err = Scenario::from_toml_str(text).unwrap_err();
        assert!(matches!(err, ScenarioError::Invalid(_)));
    }

    #[test]
    fn parses_faults_and_steps() {
        let text = r#"
name = "full"
transport = "tcp"
duration_ms = 60000

[[steps]]
kind = "catalog_query"
at_ms = 1000

[[faults]]
kind = "drop"
rate = 0.1
target = "keepalive"

[[faults]]
kind = "half_packet"
rate = 0.2

[[faults]]
kind = "sip_error"
rate = 0.3
code = 503
target = "register"
"#;
        let scenario = Scenario::from_toml_str(text).unwrap();
        assert_eq!(scenario.faults.len(), 3);
        assert_eq!(scenario.steps.len(), 1);
        assert_eq!(scenario.steps[0].at_ms(), 1000);
    }
}

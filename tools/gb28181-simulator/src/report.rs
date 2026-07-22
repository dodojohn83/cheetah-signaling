//! Reproducible run report.
//!
//! Every run emits a report binding the seed, scenario, message/fault counts,
//! semantic outcomes, resource usage and a transcript hash.  The hash is a
//! rolling digest over the ordered, payload-free semantic transcript, so two
//! runs of the same seed and scenario produce byte-identical reports.

use crate::fault::FaultCounts;
use serde::Serialize;

/// Aggregate message counts.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct MessageCounts {
    /// SIP messages emitted by devices and the platform.
    pub sent: u64,
    /// Delivery events ingested by receivers (fragments count individually).
    pub delivered: u64,
    /// Parse errors observed on ingest (malformed/truncated framing).
    pub parse_errors: u64,
}

/// Semantic outcome counters.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Outcomes {
    /// Number of digest challenges issued.
    pub challenges: u64,
    /// Number of accepted registrations.
    pub registrations_accepted: u64,
    /// Number of devices ever registered.
    pub devices_registered: u64,
    /// Keepalives emitted by devices.
    pub keepalives_sent: u64,
    /// Keepalives acknowledged by the platform.
    pub keepalives_acked: u64,
    /// Catalog responses emitted by devices.
    pub catalog_responses: u64,
    /// Catalog responses received by the platform.
    pub catalog_received: u64,
    /// INVITEs answered by devices (media control only).
    pub invites_answered: u64,
    /// BYEs answered by devices.
    pub byes_answered: u64,
    /// SIP error responses injected by the platform.
    pub errors_injected: u64,
}

/// Resource-usage summary (bounded by construction).
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Resources {
    /// Fixed number of shard workers.
    pub shard_count: u32,
    /// Bounded UDP source sockets shared across devices.
    pub udp_sockets: u32,
    /// Bounded TCP connection pool size.
    pub tcp_pool: u32,
    /// Peak simultaneously-scheduled timer-wheel events.
    pub peak_scheduled_events: u64,
    /// Total timer-wheel events processed.
    pub total_events_processed: u64,
}

/// The complete, reproducible run report.
#[derive(Clone, Debug, Serialize)]
pub struct RunReport {
    /// Scenario name.
    pub scenario_name: String,
    /// Master seed.
    pub seed: u64,
    /// Transport label.
    pub transport: String,
    /// Profile identifier.
    pub profile: String,
    /// GB/T standard label.
    pub standard: String,
    /// Whether the profile is a synthetic vendor fixture (non-interop).
    pub synthetic_vendor: bool,
    /// Number of devices.
    pub device_count: u32,
    /// Virtual duration in milliseconds.
    pub duration_ms: u64,
    /// Message counts.
    pub message_counts: MessageCounts,
    /// Fault counts.
    pub fault_counts: FaultCounts,
    /// Semantic outcomes.
    pub outcomes: Outcomes,
    /// Resource usage.
    pub resources: Resources,
    /// Number of transcript entries hashed.
    pub transcript_entries: u64,
    /// Hex-encoded SHA-256 digest of the semantic transcript.
    pub transcript_hash: String,
}

impl RunReport {
    /// Serializes the report to pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if serialization fails.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

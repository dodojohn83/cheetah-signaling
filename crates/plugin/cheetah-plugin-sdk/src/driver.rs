//! Core driver ports and context for protocol plugins.

use crate::error::PluginError;
use crate::manifest::{PluginName, ProtocolCapability, ResourceBudget};
use async_trait::async_trait;
use cheetah_signal_types::{DurationMs, TenantId, UtcTimestamp, clamp_str};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Maximum byte length of a [`CapabilityDescriptor`] protocol name.
const MAX_CAPABILITY_PROTOCOL_BYTES: usize = 64;
/// Maximum number of entries in [`CapabilityDescriptor::metadata`].
const MAX_CAPABILITY_METADATA_ENTRIES: usize = 64;
/// Maximum byte length of a [`CapabilityDescriptor`] metadata key.
const MAX_CAPABILITY_METADATA_KEY_BYTES: usize = 128;
/// Maximum byte length of a [`CapabilityDescriptor`] metadata value.
const MAX_CAPABILITY_METADATA_VALUE_BYTES: usize = 4096;
/// Maximum byte length of a [`HealthReport`] human-readable message.
const MAX_HEALTH_MESSAGE_BYTES: usize = 1024;
/// Maximum number of entries in [`HealthReport::metrics`].
const MAX_HEALTH_METRICS_ENTRIES: usize = 64;
/// Maximum byte length of a [`HealthReport`] metric key.
const MAX_HEALTH_METRIC_KEY_BYTES: usize = 128;

/// A command delivered to a protocol driver.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DriverCommand {
    /// Stable command type, e.g. `ptz` or `start_live`.
    pub command_type: String,
    /// Opaque command payload. Protocol drivers interpret their own schema.
    pub payload: serde_json::Value,
    /// Request idempotency key.
    pub idempotency_key: String,
    /// Deadline by which the command must be applied or rejected.
    pub deadline: UtcTimestamp,
}

/// An event emitted by a protocol driver.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProtocolEvent {
    /// Stable event type, e.g. `device_registered` or `alarm`.
    pub event_type: String,
    /// Opaque event payload.
    pub payload: serde_json::Value,
    /// Source tenant, if known.
    pub tenant_id: Option<TenantId>,
}

/// A monotonic second counter used by drivers for timeouts and replay windows.
///
/// Drivers must not read the system clock directly; the host provides this
/// value through [`DriverContext::monotonic_now`].
pub type MonotonicSeconds = u64;

/// Sink for protocol events emitted by a driver.
#[async_trait]
pub trait DeviceSink: Send + Sync {
    /// Publishes an event to the host message bus.
    async fn emit_event(&self, event: ProtocolEvent) -> Result<(), PluginError>;
}

/// Source of commands for a protocol driver.
#[async_trait]
pub trait CommandSource: Send + Sync {
    /// Returns the next command, or `None` if the source is closed.
    async fn next_command(&self) -> Result<Option<DriverCommand>, PluginError>;
}

/// Capabilities provided to a driver by the host.
#[async_trait]
pub trait DriverContext: Send + Sync {
    /// Plugin instance name.
    fn plugin_name(&self) -> &PluginName;
    /// Parsed plugin configuration.
    fn config(&self) -> &serde_json::Value;
    /// Resource budget the host has granted.
    fn budget(&self) -> &ResourceBudget;
    /// Monotonic second counter from the host clock.
    ///
    /// Drivers must use this instead of reading the system clock themselves.
    fn monotonic_now(&self) -> MonotonicSeconds;
    /// Event sink for this driver instance.
    fn device_sink(&self) -> &dyn DeviceSink;
    /// Command source for this driver instance.
    fn command_source(&self) -> &dyn CommandSource;
    /// Looks up a tenant-scoped secret reference.
    ///
    /// Returns `None` if the secret is not found or access is denied.
    /// The returned `SecretString` prevents accidental logging or serialization
    /// of the credential; callers must explicitly call `ExposeSecret::expose_secret`
    /// to access the plaintext.
    async fn secret(&self, name: &str) -> Result<Option<SecretString>, PluginError>;
    /// Requests a new media session from the host.
    async fn request_media_session(
        &self,
        params: serde_json::Value,
        timeout: DurationMs,
    ) -> Result<String, PluginError>;
    /// Registers a protocol endpoint.
    async fn register_endpoint(&self, protocol: &str, address: &str)
    -> Result<String, PluginError>;
}

/// Driver health status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// The driver is healthy.
    Healthy,
    /// The driver is degraded but still functioning.
    Degraded,
    /// The driver has failed and should be restarted.
    Unhealthy,
}

/// Capability descriptor returned by a driver probe.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    /// Protocol name.
    pub protocol: String,
    /// Direction supported.
    pub direction: crate::manifest::ProtocolDirection,
    /// Additional protocol-specific metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl CapabilityDescriptor {
    /// Validates that the descriptor fits within configured bounds.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.protocol.len() > MAX_CAPABILITY_PROTOCOL_BYTES {
            return Err(PluginError::Driver(format!(
                "capability protocol exceeds {} bytes",
                MAX_CAPABILITY_PROTOCOL_BYTES
            )));
        }
        if self.metadata.len() > MAX_CAPABILITY_METADATA_ENTRIES {
            return Err(PluginError::Driver(format!(
                "capability metadata exceeds {} entries",
                MAX_CAPABILITY_METADATA_ENTRIES
            )));
        }
        for (k, v) in &self.metadata {
            if k.len() > MAX_CAPABILITY_METADATA_KEY_BYTES {
                return Err(PluginError::Driver(format!(
                    "capability metadata key exceeds {} bytes",
                    MAX_CAPABILITY_METADATA_KEY_BYTES
                )));
            }
            if v.len() > MAX_CAPABILITY_METADATA_VALUE_BYTES {
                return Err(PluginError::Driver(format!(
                    "capability metadata value exceeds {} bytes",
                    MAX_CAPABILITY_METADATA_VALUE_BYTES
                )));
            }
        }
        Ok(())
    }
}

/// Health report produced by a driver.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HealthReport {
    /// Overall health status.
    pub status: HealthStatus,
    /// Human-readable message.
    pub message: String,
    /// Counters such as active devices, pending commands, etc.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metrics: HashMap<String, u64>,
}

impl HealthReport {
    /// Validates that the report fits within configured bounds.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.message.len() > MAX_HEALTH_MESSAGE_BYTES {
            return Err(PluginError::Driver(format!(
                "health message exceeds {} bytes",
                MAX_HEALTH_MESSAGE_BYTES
            )));
        }
        if self.metrics.len() > MAX_HEALTH_METRICS_ENTRIES {
            return Err(PluginError::Driver(format!(
                "health metrics exceed {} entries",
                MAX_HEALTH_METRICS_ENTRIES
            )));
        }
        for k in self.metrics.keys() {
            if k.len() > MAX_HEALTH_METRIC_KEY_BYTES {
                return Err(PluginError::Driver(format!(
                    "health metric key exceeds {} bytes",
                    MAX_HEALTH_METRIC_KEY_BYTES
                )));
            }
        }
        Ok(())
    }

    /// Returns a new `HealthReport` with the message and metrics clamped to bounds.
    pub fn clamp_to_bounds(&self) -> Self {
        let mut metrics = HashMap::new();
        for (k, v) in &self.metrics {
            let key = clamp_str(k, MAX_HEALTH_METRIC_KEY_BYTES);
            if metrics.len() >= MAX_HEALTH_METRICS_ENTRIES {
                break;
            }
            metrics.insert(key, *v);
        }
        Self {
            status: self.status,
            message: clamp_str(&self.message, MAX_HEALTH_MESSAGE_BYTES),
            metrics,
        }
    }
}

/// A protocol driver implementation.
#[async_trait]
pub trait ProtocolDriver: Send + Sync {
    /// Starts the driver. Called after successful load and negotiation.
    async fn start(&self, ctx: &dyn DriverContext, timeout: DurationMs) -> Result<(), PluginError>;
    /// Stops accepting new work and drains in-flight commands.
    async fn drain(&self, ctx: &dyn DriverContext, timeout: DurationMs) -> Result<(), PluginError>;
    /// Shuts the driver down.
    async fn shutdown(
        &self,
        ctx: &dyn DriverContext,
        timeout: DurationMs,
    ) -> Result<(), PluginError>;
    /// Handles a single command.
    async fn handle_command(
        &self,
        ctx: &dyn DriverContext,
        command: DriverCommand,
        timeout: DurationMs,
    ) -> Result<(), PluginError>;
    /// Probes a target (e.g. a device address) and returns its capabilities.
    async fn probe(
        &self,
        ctx: &dyn DriverContext,
        target: &str,
        timeout: DurationMs,
    ) -> Result<CapabilityDescriptor, PluginError>;
    /// Returns a health report.
    async fn health(
        &self,
        ctx: &dyn DriverContext,
        timeout: DurationMs,
    ) -> Result<HealthReport, PluginError>;
}

/// Factory for creating protocol driver instances.
#[async_trait]
pub trait ProtocolDriverFactory: Send + Sync {
    /// Plugin name produced by this factory.
    fn name(&self) -> PluginName;
    /// Capabilities advertised by drivers from this factory.
    fn capabilities(&self) -> Vec<ProtocolCapability>;
    /// Returns the maximum time the host should allow for `create` to finish.
    ///
    /// Built-in factories that create in-process drivers can keep the default
    /// (5 seconds). Out-of-process factories should return a budget that covers
    /// process startup and the first gRPC connection.
    fn creation_timeout(&self) -> DurationMs {
        DurationMs::from_seconds(5)
    }
    /// Creates a new driver instance with the given configuration.
    async fn create(
        &self,
        config: serde_json::Value,
    ) -> Result<Box<dyn ProtocolDriver>, PluginError>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::manifest::ProtocolDirection;

    fn valid_descriptor() -> CapabilityDescriptor {
        CapabilityDescriptor {
            protocol: "onvif".to_string(),
            direction: ProtocolDirection::Outbound,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn capability_descriptor_rejects_oversized_protocol() {
        let mut descriptor = valid_descriptor();
        descriptor.protocol = "x".repeat(MAX_CAPABILITY_PROTOCOL_BYTES + 1);
        assert!(descriptor.validate().is_err());
    }

    #[test]
    fn capability_descriptor_rejects_too_many_metadata_entries() {
        let mut descriptor = valid_descriptor();
        for i in 0..MAX_CAPABILITY_METADATA_ENTRIES + 1 {
            descriptor
                .metadata
                .insert(format!("key-{i}"), "v".to_string());
        }
        assert!(descriptor.validate().is_err());
    }

    #[test]
    fn capability_descriptor_rejects_oversized_metadata_value() {
        let mut descriptor = valid_descriptor();
        descriptor.metadata.insert(
            "summary".to_string(),
            "x".repeat(MAX_CAPABILITY_METADATA_VALUE_BYTES + 1),
        );
        assert!(descriptor.validate().is_err());
    }

    #[test]
    fn health_report_clamps_message_and_metrics() {
        let report = HealthReport {
            status: HealthStatus::Healthy,
            message: "x".repeat(MAX_HEALTH_MESSAGE_BYTES + 10),
            metrics: [(
                "long_key_".to_string() + &"x".repeat(MAX_HEALTH_METRIC_KEY_BYTES + 1),
                1,
            )]
            .into_iter()
            .collect(),
        };
        let clamped = report.clamp_to_bounds();
        assert!(clamped.message.len() <= MAX_HEALTH_MESSAGE_BYTES);
        assert!(clamped.message.is_char_boundary(clamped.message.len()));
        assert!(
            clamped.metrics.is_empty() || {
                let (k, _) = clamped.metrics.iter().next().unwrap();
                k.len() <= MAX_HEALTH_METRIC_KEY_BYTES
            }
        );
    }
}

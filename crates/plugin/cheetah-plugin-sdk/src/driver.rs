//! Core driver ports and context for protocol plugins.

use crate::error::PluginError;
use crate::manifest::{PluginName, ProtocolCapability, ResourceBudget};
use async_trait::async_trait;
use cheetah_signal_types::{DurationMs, TenantId, UtcTimestamp};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// A protocol driver implementation.
#[async_trait]
pub trait ProtocolDriver: Send + Sync {
    /// Starts the driver. Called after successful load and negotiation.
    async fn start(&self, ctx: &dyn DriverContext) -> Result<(), PluginError>;
    /// Stops accepting new work and drains in-flight commands.
    async fn drain(&self, ctx: &dyn DriverContext, timeout: DurationMs) -> Result<(), PluginError>;
    /// Shuts the driver down.
    async fn shutdown(&self, ctx: &dyn DriverContext) -> Result<(), PluginError>;
    /// Handles a single command.
    async fn handle_command(
        &self,
        ctx: &dyn DriverContext,
        command: DriverCommand,
    ) -> Result<(), PluginError>;
    /// Probes a target (e.g. a device address) and returns its capabilities.
    async fn probe(&self, target: &str) -> Result<CapabilityDescriptor, PluginError>;
    /// Returns a health report.
    async fn health(&self) -> HealthReport;
}

/// Factory for creating protocol driver instances.
#[async_trait]
pub trait ProtocolDriverFactory: Send + Sync {
    /// Plugin name produced by this factory.
    fn name(&self) -> PluginName;
    /// Capabilities advertised by drivers from this factory.
    fn capabilities(&self) -> Vec<ProtocolCapability>;
    /// Creates a new driver instance with the given configuration.
    async fn create(
        &self,
        config: serde_json::Value,
    ) -> Result<Box<dyn ProtocolDriver>, PluginError>;
}

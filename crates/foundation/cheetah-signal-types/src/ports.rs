//! Port traits for the foundation layer.

use crate::{
    ChannelId, CorrelationId, DeviceId, DurationMs, EndpointId, EventId, MediaSessionId, MessageId,
    NodeId, OperationId, PluginId, ProtocolSessionId, Result, TenantId, UtcTimestamp,
};
use secrecy::SecretString;

/// Source of wall time and monotonic deadlines.
///
/// Implementations can be swapped for test clocks.
pub trait Clock: Send + Sync {
    /// Returns the current wall time in UTC.
    fn now_wall(&self) -> UtcTimestamp;
    /// Returns the current monotonic time.
    fn now_monotonic(&self) -> DurationMs;
}

/// Generates well formed identifiers.
///
/// Implementations can be deterministic in tests.
pub trait IdGenerator: Send + Sync {
    /// Generates a new tenant identifier.
    fn generate_tenant_id(&self) -> TenantId;
    /// Generates a new device identifier.
    fn generate_device_id(&self) -> DeviceId;
    /// Generates a new endpoint identifier.
    fn generate_endpoint_id(&self) -> EndpointId;
    /// Generates a new channel identifier.
    fn generate_channel_id(&self) -> ChannelId;
    /// Generates a new protocol session identifier.
    fn generate_protocol_session_id(&self) -> ProtocolSessionId;
    /// Generates a new media session identifier.
    fn generate_media_session_id(&self) -> MediaSessionId;
    /// Generates a new operation identifier.
    fn generate_operation_id(&self) -> OperationId;
    /// Generates a new node identifier.
    fn generate_node_id(&self) -> NodeId;
    /// Generates a new plugin identifier.
    fn generate_plugin_id(&self) -> PluginId;
    /// Generates a new event identifier.
    fn generate_event_id(&self) -> EventId;
    /// Generates a new message identifier.
    fn generate_message_id(&self) -> MessageId;
    /// Generates a new correlation identifier.
    fn generate_correlation_id(&self) -> CorrelationId;
}

/// Secure secret storage accessed by reference.
///
/// No plaintext enumeration interface is exposed.
pub trait SecretStore: Send + Sync {
    /// Retrieves a secret by its reference key.
    fn get(&self, key: &str) -> Result<SecretString>;
    /// Stores a secret under a reference key.
    fn put(&self, key: &str, value: SecretString) -> Result<()>;
    /// Deletes a secret by reference.
    fn delete(&self, key: &str) -> Result<()>;
    /// Rotates a secret, returning the previous value.
    fn rotate(&self, key: &str) -> Result<SecretString>;
}

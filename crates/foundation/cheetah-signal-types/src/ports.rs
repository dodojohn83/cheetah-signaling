//! Port traits for the foundation layer.

use crate::{
    ChannelId, CorrelationId, DeliveryId, DeviceId, DurationMs, EndpointId, EventId,
    MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, NodeInstanceId,
    OperationId, PluginId, ProtocolSessionId, Result, TenantId, UtcTimestamp, WebhookId,
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
    /// Generates a new media binding identifier.
    fn generate_media_binding_id(&self) -> MediaBindingId;
    /// Generates a new media node instance epoch.
    fn generate_media_node_instance_epoch(&self) -> MediaNodeInstanceEpoch;
    /// Generates a new operation identifier.
    fn generate_operation_id(&self) -> OperationId;
    /// Generates a new node identifier.
    fn generate_node_id(&self) -> NodeId;
    /// Generates a new node instance identifier.
    fn generate_node_instance_id(&self) -> NodeInstanceId;
    /// Generates a new plugin identifier.
    fn generate_plugin_id(&self) -> PluginId;
    /// Generates a new event identifier.
    fn generate_event_id(&self) -> EventId;
    /// Generates a new message identifier.
    fn generate_message_id(&self) -> MessageId;
    /// Generates a new correlation identifier.
    fn generate_correlation_id(&self) -> CorrelationId;
    /// Generates a new webhook identifier.
    fn generate_webhook_id(&self) -> WebhookId;
    /// Generates a new webhook delivery identifier.
    fn generate_delivery_id(&self) -> DeliveryId;
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

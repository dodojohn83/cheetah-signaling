//! Port traits for the foundation layer.

use crate::{
    ChannelId, CorrelationId, DeliveryId, DeviceId, DurationMs, EndpointId, EventId,
    MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, OperationId,
    PluginId, ProtocolSessionId, Result, TenantId, UtcTimestamp, WebhookId,
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

/// Source of randomness that can be seeded for deterministic tests.
///
/// Production implementations use the operating system's CSPRNG. Test
/// implementations can use a fixed seed so failing tests are reproducible.
pub trait RandomSource: Send + Sync {
    /// Fills `dest` with random bytes.
    fn fill_bytes(&self, dest: &mut [u8]);

    /// Returns a random `u64`.
    fn u64(&self) -> u64;

    /// Returns a random `usize` in the range `0..bound`.
    ///
    /// # Panics
    ///
    /// Panics if `bound` is zero.
    fn usize(&self, bound: usize) -> usize;
}

/// Policy consulted by network adapters to inject deterministic faults.
///
/// This port stays in `cheetah-signal-types` so protocol core and driver
/// tests can model delay, drop, and reorder without depending on a specific
/// runtime.
pub trait NetworkFaultPolicy: Send + Sync {
    /// Whether the next outbound packet should be dropped.
    fn should_drop(&self) -> bool;

    /// Additional latency to add before the next packet, in milliseconds.
    fn latency_ms(&self) -> u64;
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

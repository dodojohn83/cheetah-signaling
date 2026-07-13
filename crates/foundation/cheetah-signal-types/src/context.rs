//! Request context, principal, and resource reference types.

use crate::{
    ChannelId, CorrelationId, Deadline, DeviceId, EndpointId, EventId, MediaSessionId, MessageId,
    NodeId, OperationId, ProtocolSessionId, TenantId,
};

/// Authenticated actor making a request.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Principal {
    /// Identity of the actor.
    pub id: String,
    /// Kind of principal (user, service, plugin, device, ...).
    pub kind: PrincipalKind,
    /// Granted scopes for this request.
    pub scopes: Vec<String>,
}

/// Kind of authenticated principal.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PrincipalKind {
    /// Human or service user.
    User,
    /// Internal service.
    Service,
    /// Plugin instance.
    Plugin,
    /// Device.
    Device,
    /// Unknown principal kind.
    #[default]
    Unknown,
}

/// Context carried with every request.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RequestContext {
    /// Tenant of the request.
    pub tenant_id: TenantId,
    /// Principal making the request.
    pub principal: Principal,
    /// Request message identifier.
    pub message_id: MessageId,
    /// Correlation identifier for cross-service tracing.
    pub correlation_id: CorrelationId,
    /// W3C trace parent.
    pub traceparent: Option<String>,
    /// W3C trace state.
    pub tracestate: Option<String>,
    /// Deadline by which the request must complete.
    pub deadline: Option<Deadline>,
    /// Node handling the request.
    pub node_id: Option<NodeId>,
}

/// A reference to a resource in the system.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ResourceRef {
    /// Tenant of the resource.
    pub tenant_id: TenantId,
    /// Resource kind.
    pub kind: ResourceKind,
    /// Resource identifier, scope depends on kind.
    pub id: ResourceId,
}

/// Kind of resource being referenced.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResourceKind {
    /// Tenant.
    Tenant,
    /// Device.
    Device,
    /// Channel.
    Channel,
    /// Endpoint.
    Endpoint,
    /// Protocol session.
    ProtocolSession,
    /// Media session.
    MediaSession,
    /// Operation.
    Operation,
    /// Event.
    Event,
    /// Plugin.
    Plugin,
    /// Node.
    Node,
}

/// Identifier for a resource, grouped by the resource kind.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResourceId {
    /// Tenant scoped.
    Tenant(TenantId),
    /// Device scoped.
    Device(DeviceId),
    /// Channel scoped.
    Channel(ChannelId),
    /// Endpoint scoped.
    Endpoint(EndpointId),
    /// Protocol session scoped.
    ProtocolSession(ProtocolSessionId),
    /// Media session scoped.
    MediaSession(MediaSessionId),
    /// Operation scoped.
    Operation(OperationId),
    /// Event scoped.
    Event(EventId),
    /// Plugin scoped.
    Plugin(crate::PluginId),
    /// Node scoped.
    Node(NodeId),
}

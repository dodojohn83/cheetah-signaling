//! Request context, principal, and resource reference types.

use crate::trace_context::{validate_traceparent, validate_tracestate};
use crate::{
    ChannelId, CorrelationId, Deadline, DeviceId, EndpointId, EventId, MediaBindingId,
    MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, OperationId, OwnerEpoch,
    ProtocolSessionId, SignalError, SignalErrorKind, TenantId, UtcTimestamp,
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
    /// Source IP address of the request when available.
    pub source_ip: Option<String>,
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
    /// Media binding.
    MediaBinding,
    /// Operation.
    Operation,
    /// Event.
    Event,
    /// Plugin.
    Plugin,
    /// Node.
    Node,
    /// Media node (media plane worker).
    MediaNode,
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
    /// Media binding scoped.
    MediaBinding(MediaBindingId),
    /// Operation scoped.
    Operation(OperationId),
    /// Event scoped.
    Event(EventId),
    /// Plugin scoped.
    Plugin(crate::PluginId),
    /// Node scoped.
    Node(NodeId),
    /// Media node scoped.
    MediaNode(NodeId),
}

/// Context carried with every mutating media request.
///
/// This is the domain-side mirror of `cheetah.media.v1.MediaMutationContext`.
/// Validation is explicit and must run before any side effect is applied.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MediaMutationContext {
    /// Tenant that owns the request.
    pub tenant_id: TenantId,
    /// Request identifier for tracing.
    pub request_id: String,
    /// Cross-service correlation identifier.
    pub correlation_id: CorrelationId,
    /// Message identifier used for inbox/event de-duplication.
    pub message_id: MessageId,
    /// Idempotency key for the operation.
    pub idempotency_key: String,
    /// Absolute UTC deadline after which no side effects should be applied.
    pub deadline: Option<Deadline>,
    /// Signaling node that owns the device/session.
    pub source_signaling_node_id: NodeId,
    /// Owner epoch used to fence stale device owners.
    pub owner_epoch: OwnerEpoch,
    /// Target media node for the operation.
    pub target_media_node_id: NodeId,
    /// Media process instance epoch for fencing.
    pub target_media_node_instance_epoch: MediaNodeInstanceEpoch,
    /// Saga operation identifier.
    pub operation_id: OperationId,
    /// Saga step identifier.
    pub operation_step_id: String,
    /// Logical media session when one exists.
    pub media_session_id: Option<MediaSessionId>,
    /// Physical media binding when one exists.
    pub media_binding_id: Option<MediaBindingId>,
    /// Contract version negotiated between signaling and media.
    pub contract_version: u64,
    /// W3C trace parent.
    pub traceparent: Option<String>,
    /// W3C trace state.
    pub tracestate: Option<String>,
}

impl MediaMutationContext {
    /// Validates the context relative to `now`.
    ///
    /// Returns `InvalidArgument` when required fields are missing, when the
    /// deadline has already passed, or when trace context values are malformed.
    pub fn validate(&self, now: UtcTimestamp) -> crate::Result<()> {
        if self.tenant_id.as_uuid().is_nil() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "tenant_id is required",
            )
            .with_field_violation("tenant_id", "must not be nil"));
        }
        if self.request_id.is_empty() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "request_id is required",
            )
            .with_field_violation("request_id", "must be non-empty"));
        }
        if self.correlation_id.as_uuid().is_nil() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "correlation_id is required",
            )
            .with_field_violation("correlation_id", "must not be nil"));
        }
        if self.message_id.as_uuid().is_nil() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "message_id is required",
            )
            .with_field_violation("message_id", "must not be nil"));
        }
        if self.operation_id.as_uuid().is_nil() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "operation_id is required",
            )
            .with_field_violation("operation_id", "must not be nil"));
        }
        if self.operation_step_id.is_empty() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "operation_step_id is required",
            )
            .with_field_violation("operation_step_id", "must be non-empty"));
        }
        if self.idempotency_key.is_empty() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "idempotency_key is required",
            )
            .with_field_violation("idempotency_key", "must be non-empty"));
        }
        let deadline = self.deadline.ok_or_else(|| {
            SignalError::new(SignalErrorKind::InvalidArgument, "deadline is required")
                .with_field_violation("deadline", "must be set")
        })?;
        if deadline.is_elapsed(now) {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "deadline has already passed",
            )
            .with_field_violation("deadline", "must be in the future"));
        }
        if self.owner_epoch.0 == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "owner_epoch must be set",
            )
            .with_field_violation("owner_epoch", "must be greater than zero"));
        }
        if self.source_signaling_node_id.as_uuid().is_nil() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "source_signaling_node_id must be a valid node id",
            )
            .with_field_violation("source_signaling_node_id", "must not be nil"));
        }
        if self.target_media_node_id.as_uuid().is_nil() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "target_media_node_id must be a valid node id",
            )
            .with_field_violation("target_media_node_id", "must not be nil"));
        }
        if self.target_media_node_instance_epoch.0 == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "target_media_node_instance_epoch must be set",
            )
            .with_field_violation(
                "target_media_node_instance_epoch",
                "must be greater than zero",
            ));
        }
        if self.contract_version == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "contract_version must be set",
            )
            .with_field_violation("contract_version", "must be greater than zero"));
        }
        if let Some(ref tp) = self.traceparent
            && validate_traceparent(tp).is_none()
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "traceparent is malformed",
            )
            .with_field_violation("traceparent", "must match W3C format"));
        }
        if let Some(ref ts) = self.tracestate
            && validate_tracestate(ts).is_none()
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "tracestate is malformed",
            )
            .with_field_violation("tracestate", "must match W3C format"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Clock;
    use crate::test_support::FakeClock;
    use crate::time::DurationMs;

    fn valid_context(clock: &FakeClock) -> MediaMutationContext {
        MediaMutationContext {
            tenant_id: TenantId::generate(),
            request_id: "request-1".to_string(),
            correlation_id: CorrelationId::generate(),
            message_id: MessageId::generate(),
            idempotency_key: "idem-1".to_string(),
            deadline: Deadline::from_now(clock.now_wall(), DurationMs::from_millis(1000)),
            source_signaling_node_id: NodeId::generate(),
            owner_epoch: OwnerEpoch(1),
            target_media_node_id: NodeId::generate(),
            target_media_node_instance_epoch: MediaNodeInstanceEpoch(1),
            operation_id: OperationId::generate(),
            operation_step_id: "step-1".to_string(),
            media_session_id: None,
            media_binding_id: None,
            contract_version: 1,
            traceparent: None,
            tracestate: None,
        }
    }

    #[test]
    fn valid_context_passes() {
        let clock = FakeClock::new();
        assert!(valid_context(&clock).validate(clock.now_wall()).is_ok());
    }

    #[test]
    fn nil_tenant_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.tenant_id = TenantId::from_uuid(uuid::Uuid::nil());
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn missing_idempotency_key_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.idempotency_key.clear();
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn missing_deadline_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.deadline = None;
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn elapsed_deadline_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.deadline = Deadline::from_now(clock.now_wall(), DurationMs::from_millis(-1));
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn missing_owner_epoch_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.owner_epoch = OwnerEpoch(0);
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn nil_source_node_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.source_signaling_node_id = NodeId::from_uuid(uuid::Uuid::nil());
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn empty_request_id_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.request_id.clear();
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn nil_correlation_id_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.correlation_id = CorrelationId::from_uuid(uuid::Uuid::nil());
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn nil_message_id_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.message_id = MessageId::from_uuid(uuid::Uuid::nil());
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn nil_operation_id_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.operation_id = OperationId::from_uuid(uuid::Uuid::nil());
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn empty_operation_step_id_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.operation_step_id.clear();
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }

    #[test]
    fn malformed_traceparent_fails() {
        let clock = FakeClock::new();
        let mut ctx = valid_context(&clock);
        ctx.traceparent = Some("invalid".to_string());
        let Err(err) = ctx.validate(clock.now_wall()) else {
            panic!("expected validation to fail");
        };
        assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    }
}

//! Foundation types for Cheetah Signaling.
//!
//! This crate provides shared newtypes, timestamps, errors, pagination,
//! request context, and configuration models used by the rest of the workspace.
//! It must not depend on Tokio, Axum, Tonic, SQLx, async-nats or other
//! runtime or adapter crates.

#![doc = include_str!("../README.md")]

pub mod admission;
pub mod audit;
pub mod config;
pub mod context;
pub mod error;
pub mod event;
pub mod gb_metrics;
pub mod id;
pub mod metrics;
pub mod net;
pub mod pagination;
pub mod ports;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod time;
pub mod trace_context;

pub use admission::{
    BacklogController, BacklogObservation, BacklogState, CoalesceDecision, Coalescer,
    DeadLetterEntry, DeadLetterQueue, DeadLetterReason, KeyedRateLimiter, Priority, TokenBucket,
    TokenBucketConfig, TrafficClass,
};
pub use audit::{AuditEvent, AuditLog, AuditOutcome, NoOpAuditLog};
pub use config::{ConfigSource, DeploymentProfile, SignalConfig};
pub use context::{
    MediaMutationContext, Principal, PrincipalKind, RequestContext, ResourceId, ResourceKind,
    ResourceRef,
};
pub use error::{FieldViolation, Result, SignalError, SignalErrorKind};
pub use event::Event;
pub use gb_metrics::{
    GbCommandMethod, GbCommandOutcome, GbDevicePresence, GbMediaSessionState, GbMetricsRecorder,
    NoopGbMetricsRecorder,
};
pub use id::{
    ChannelId, CorrelationId, DeliveryId, DeviceId, EndpointId, EventId, MediaBindingId,
    MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, NodeInstanceId, OperationId,
    OwnerEpoch, PluginId, ProtocolIdentity, ProtocolSessionId, Revision, TenantId, WebhookId,
};
pub use metrics::MetricsExporter;
pub use net::is_internal_ip;
pub use pagination::{DEFAULT_PAGE_SIZE, ListCursor, MAX_PAGE_SIZE, Page, PageRequest};
pub use ports::{Clock, IdGenerator, NetworkFaultPolicy, RandomSource, SecretStore};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{
    FakeClock, FakeIdGenerator, FakeNetworkFault, FakeRandom, NoOpNetworkFault, TestSeed,
};
pub use time::{Deadline, DurationMs, UtcTimestamp};
pub use trace_context::{validate_traceparent, validate_tracestate};

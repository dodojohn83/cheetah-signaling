//! Foundation types for Cheetah Signaling.
//!
//! This crate provides shared newtypes, timestamps, errors, pagination,
//! request context, and configuration models used by the rest of the workspace.
//! It must not depend on Tokio, Axum, Tonic, SQLx, async-nats or other
//! runtime or adapter crates.

#![doc = include_str!("../README.md")]

pub mod config;
pub mod context;
pub mod error;
pub mod event;
pub mod id;
pub mod pagination;
pub mod ports;
pub mod time;

pub use config::{ConfigSource, SignalConfig};
pub use context::{
    Principal, PrincipalKind, RequestContext, ResourceId, ResourceKind, ResourceRef,
};
pub use error::{FieldViolation, Result, SignalError, SignalErrorKind};
pub use event::Event;
pub use id::{
    ChannelId, CorrelationId, DeviceId, EndpointId, EventId, MediaBindingId,
    MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, OperationId, OwnerEpoch, PluginId,
    ProtocolIdentity, ProtocolSessionId, Revision, TenantId,
};
pub use pagination::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, Page, PageRequest};
pub use ports::{Clock, IdGenerator, SecretStore};
pub use time::{Deadline, DurationMs, UtcTimestamp};

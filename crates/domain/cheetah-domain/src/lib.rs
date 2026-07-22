//! Domain layer for Cheetah Signaling.
//!
//! This crate contains the authoritative aggregates, value objects, ports and
//! in-memory test fixtures. It does not depend on Tokio, Axum, Tonic, SQLx,
//! async-nats or concrete protocol crates.

#![doc = include_str!("../README.md")]

pub mod channel;
pub mod command;
pub mod device;
pub mod error;
pub mod event;
pub mod media_binding;
pub mod media_callback;
pub mod media_event_handler;
pub mod media_key;
pub mod media_node;
pub mod media_session;
pub mod node;
pub mod operation;
pub mod platform_link;
pub mod ports;
pub mod protocol_session;
pub mod tenant;
pub mod webhook;

#[cfg(any(test, feature = "test-util"))]
pub mod in_memory;

pub use channel::{
    Channel, ChannelKind, ChannelStatus, PresetAction, PtzCapabilities, StreamProfile,
};
pub use command::{
    Command, CommandPayload, DeviceControlCommand, DeviceControlKind, IdempotencyScope,
    MediaControl, PresetCommand, PtzDirection, QueryCommand, QueryKind,
};
pub use device::{
    Capability, CapabilityValue, Connectivity, Device, DeviceKind, DeviceLifecycle, Protocol,
};
pub use error::DomainError;
pub use event::DomainEvent;
pub use media_binding::{MediaBinding, MediaBindingError, MediaBindingState};
pub use media_callback::{MediaNodeCallback, MediaNodeCallbackKind, MediaNodeSessionRef};
pub use media_event_handler::MediaEventHandler;
pub use media_key::MediaKey;
pub use media_node::{MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus};
pub use media_session::{
    MediaPurpose, MediaSession, MediaSessionDesiredState, MediaSessionError, MediaSessionState,
};
pub use node::{ClusterNode, NodeCapacity, NodeLoad};
pub use operation::{
    DispatchAttempt, DispatchAttemptStatus, Operation, OperationError, OperationResult,
    OperationStatus, OperationStep, OperationStepStatus,
};
pub use platform_link::{
    ActualRegistrationState, BackoffPolicy, DesiredRegistrationState, GbPlatformLink,
    MAX_CASCADE_HOPS, NewPlatformLink, PlatformAcl, PlatformCredential, PlatformDirection,
    PlatformEndpoint, PlatformIdentityPair, RegistrationRuntime, SubscriptionLimits, detect_loop,
};
pub use ports::*;
pub use protocol_session::{
    CompatibilityCapability, CompatibilityProfile, LocalIdentity, NewProtocolSession,
    PresenceState, ProfileSelector, ProtocolSession, RegistrationInfo, SessionEndpoint,
    SipTransport,
};
pub use tenant::{MAX_TENANT_NAME_LEN, Tenant};
pub use webhook::{DeliveryStatus, WebhookConfig, WebhookDelivery, sign_webhook_payload};

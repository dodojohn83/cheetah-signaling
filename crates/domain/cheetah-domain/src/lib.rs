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
pub mod media_session;
pub mod operation;
pub mod ports;
pub mod webhook;

#[cfg(any(test, feature = "test-util"))]
pub mod in_memory;

pub use channel::{
    Channel, ChannelKind, ChannelStatus, PresetAction, PtzCapabilities, StreamProfile,
};
pub use command::{Command, CommandPayload, IdempotencyScope, MediaControl, PtzDirection};
pub use device::{
    Capability, CapabilityValue, Connectivity, Device, DeviceKind, DeviceLifecycle, Protocol,
};
pub use error::DomainError;
pub use event::DomainEvent;
pub use media_binding::{MediaBinding, MediaBindingError, MediaBindingState};
pub use media_session::{
    MediaPurpose, MediaSession, MediaSessionDesiredState, MediaSessionError, MediaSessionState,
};
pub use operation::{Operation, OperationError, OperationResult, OperationStatus};
pub use ports::*;
pub use webhook::{
    sign_webhook_payload, DeliveryStatus, WebhookConfig, WebhookDelivery,
};

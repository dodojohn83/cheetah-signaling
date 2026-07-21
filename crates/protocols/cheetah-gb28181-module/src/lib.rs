//! GB28181 protocol module: maps SIP/GB XML wire messages to domain events.
//!
//! This crate is Sans-I/O. It produces `AccessOutput` values that a driver or
//! application layer must execute (send a SIP response, publish an event, etc.).
//!
//! # Crate boundaries
//!
//! - `cheetah-gb28181-core` provides the SIP/Digest state machines and the
//!   `GbAccessMachine` input/output contract.
//! - This module adds GB28181 business logic: tenant/realm selection, device
//!   identity validation, authentication and command/event mapping.
//! - `cheetah-gb28181-driver-tokio` handles UDP/TCP sockets and timer
//!   injection by driving any `GbAccessMachine` implementation.

#![warn(missing_docs)]

pub mod access;
pub mod cascade;
pub mod compat;
pub mod config;
pub mod error;
pub mod events;
pub mod media;
pub mod ports;
mod registration;
pub mod types;
pub mod xml;

pub use access::Gb28181Access;
pub use cascade::{
    CascadeConfig, CascadeCredentialProvider, CascadeError, CascadeEvent, CascadeInput,
    CascadeOutput, Gb28181Cascade,
};
pub use cheetah_gb28181_core::{AccessInput, AccessOutput, GbAccessMachine};
pub use compat::{
    CatalogOverrides, CharsetPreference, CompatibilityCapability, CompatibilityError,
    CompatibilityProfile, CompatibilityProfileConfig, CompatibilityRegistry, DeviceDescriptor,
    DigestAlgorithmPreference, EndpointBehavior, MatchSpecificity, MediaStatusOverrides,
    PinnedProfile, ProfileId, ProfileMatchKey, ProfileRevision, ProfileSelection, RportPolicy,
    SdpOverrides, SdpSetupPreference, SourceRoutePolicy, StandardVersion,
};
pub use config::{AuthPolicy, CharsetPolicy, Gb28181DomainConfig};
pub use error::AccessError;
pub use events::{DevicePresence, Gb28181Event};
pub use media::{
    Gb28181Media, MediaCommand, MediaConfig, MediaError, MediaInput, MediaOutput, MediaTransport,
    PlaybackAction,
};
pub use ports::{CredentialError, CredentialProvider};
pub use types::{DeviceId, DomainId};
pub use xml::{KeepaliveInfo, XmlElement, XmlLimits, encode_xml, parse_keepalive, parse_xml};

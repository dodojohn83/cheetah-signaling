//! Sans-I/O ONVIF service request builders, response parsers, and wire types.
//!
//! Shared by `cheetah-onvif-module` (protocol module) and
//! `cheetah-onvif-driver-tokio` (protocol driver) so the driver does not have
//! to depend on the module crate.

#![warn(missing_docs)]

pub mod config;
pub mod error;
pub mod services;
pub mod types;

pub use config::{
    AuthPolicy, MediaPreference, OnvifConfig, ParserLimits, PullPointConfig, SnapshotConfig,
    XAddrPolicy,
};
pub use error::OnvifServiceError;
pub use types::{
    CapabilityKind, CapabilityProbeResult, DeviceInformation, OnvifEvent, ProvisioningStage,
    Service,
};

//! ONVIF protocol module: maps ONVIF service requests/responses to internal
//! signaling types and defines Sans-I/O ports for the driver.
//!
//! See the crate README for allowed and forbidden dependencies.

#![warn(missing_docs)]

pub mod config;
pub mod error;
pub mod ports;
pub mod services;
pub mod types;
pub mod workflow;

pub use config::{
    AuthPolicy, MediaPreference, OnvifConfig, ParserLimits, PullPointConfig, SnapshotConfig,
    XAddrPolicy,
};
pub use error::OnvifModuleError;
pub use ports::CredentialProvider;
pub use types::{
    CapabilityKind, CapabilityProbeResult, DeviceInformation, OnvifEvent, ProvisioningStage,
    Service,
};
pub use workflow::{Provisioner, ProvisionerError, ProvisioningInput, ProvisioningOutput};

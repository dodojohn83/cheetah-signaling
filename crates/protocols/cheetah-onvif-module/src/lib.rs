//! ONVIF protocol module: maps ONVIF service requests/responses to internal
//! signaling types, defines Sans-I/O ports for the driver, and exposes a
//! built-in [`ProtocolDriver`] adapter for the plugin host.
//!
//! See the crate README for allowed and forbidden dependencies.

#![warn(missing_docs)]

pub mod config;
pub mod driver;
pub mod error;
pub mod ports;
pub mod services;
pub mod types;
pub mod workflow;

pub use config::{
    AuthPolicy, MediaPreference, OnvifConfig, ParserLimits, PullPointConfig, SnapshotConfig,
    XAddrPolicy,
};
pub use driver::{OnvifDriverFactory, OnvifProtocolDriver};
pub use error::OnvifModuleError;
pub use ports::CredentialProvider;
pub use types::{
    CapabilityKind, CapabilityProbeResult, DeviceInformation, OnvifEvent, ProvisioningStage,
    Service,
};
pub use workflow::{Provisioner, ProvisionerError, ProvisioningInput, ProvisioningOutput};

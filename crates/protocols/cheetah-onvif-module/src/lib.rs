//! ONVIF protocol module: maps ONVIF service requests/responses to internal
//! signaling types, defines Sans-I/O ports for the driver, and exposes a
//! built-in [`ProtocolDriver`] adapter for the plugin host.
//!
//! Wire-level service builders, parsers, and types live in
//! `cheetah-onvif-services` so the tokio driver can share them without
//! depending on this module crate.
//!
//! See the crate README for allowed and forbidden dependencies.

#![warn(missing_docs)]

pub use cheetah_onvif_services::{config, error, services, types};

pub mod driver;
pub mod ports;
pub mod workflow;

pub use cheetah_onvif_services::config::{
    AuthPolicy, MediaPreference, OnvifConfig, ParserLimits, PullPointConfig, SnapshotConfig,
    XAddrPolicy,
};
pub use cheetah_onvif_services::error::OnvifServiceError as OnvifModuleError;
pub use cheetah_onvif_services::types::{
    CapabilityKind, CapabilityProbeResult, DeviceInformation, OnvifEvent, ProvisioningStage,
    Service,
};

pub use driver::{OnvifDriverFactory, OnvifProtocolDriver};
pub use ports::CredentialProvider;
pub use workflow::{Provisioner, ProvisionerError, ProvisioningInput, ProvisioningOutput};

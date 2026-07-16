//! Shared ONVIF module value types and events.

use cheetah_signal_types::{ChannelId, DeviceId, MediaSessionId, TenantId};

/// Device information returned by `GetDeviceInformation`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceInformation {
    /// Manufacturer name.
    pub manufacturer: String,
    /// Model name.
    pub model: String,
    /// Firmware version.
    pub firmware_version: String,
    /// Serial number.
    pub serial_number: String,
    /// Hardware ID.
    pub hardware_id: String,
}

/// ONVIF service entry from `GetServices`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Service {
    /// Namespace of the service.
    pub namespace: String,
    /// Service endpoint URL.
    pub xaddr: String,
    /// Service version.
    pub version: String,
}

/// High-level capability kind discovered from `GetCapabilities`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CapabilityKind {
    /// Device management.
    Device,
    /// Media or Media2.
    Media,
    /// PTZ.
    Ptz,
    /// Events.
    Events,
    /// Imaging.
    Imaging,
    /// Analytics.
    Analytics,
    /// Extension / vendor capability.
    Extension,
}

impl std::fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Device => f.write_str("device"),
            Self::Media => f.write_str("media"),
            Self::Ptz => f.write_str("ptz"),
            Self::Events => f.write_str("events"),
            Self::Imaging => f.write_str("imaging"),
            Self::Analytics => f.write_str("analytics"),
            Self::Extension => f.write_str("extension"),
        }
    }
}

/// Result of probing a single capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilityProbeResult {
    /// Capability is supported with the given service namespace and endpoint.
    Supported {
        /// Service namespace.
        namespace: String,
        /// Service endpoint URL, if known.
        xaddr: Option<String>,
        /// Service version, if known.
        version: Option<String>,
    },
    /// Capability is explicitly unsupported.
    Unsupported,
    /// Probing failed with a reason and whether it is retryable.
    Failed {
        /// Reason string.
        reason: String,
        /// Whether the failure is retryable.
        retryable: bool,
    },
}

/// Provisioning stage for an ONVIF device.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProvisioningStage {
    /// Device was discovered but not yet approved.
    #[default]
    Discovered,
    /// Approval pending.
    PendingApproval,
    /// Probing services and capabilities.
    Probing,
    /// Fetching profiles and channels.
    FetchingProfiles,
    /// Generating internal device/channel records.
    GeneratingEntities,
    /// Active and usable.
    Active,
    /// Failed with a reason.
    Failed,
}

/// Events emitted by the ONVIF module for downstream consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OnvifEvent {
    /// Device information was received.
    DeviceInformationReceived {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Parsed device information.
        info: DeviceInformation,
    },
    /// A capability probe completed.
    CapabilityProbed {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Capability kind.
        kind: CapabilityKind,
        /// Probe result.
        result: CapabilityProbeResult,
    },
    /// A media session was requested.
    MediaSessionRequested {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Channel identifier.
        channel_id: ChannelId,
        /// Media session identifier.
        media_session_id: MediaSessionId,
    },
}

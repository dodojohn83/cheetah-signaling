//! GB28181 metric labels and the application-facing recorder port.
//!
//! Prometheus label cardinality must stay bounded regardless of how many
//! tenants, devices, channels or sessions exist. High-cardinality identifiers
//! (tenant/device/session IDs) are therefore never used as labels; instead the
//! label space is restricted to the fixed categories defined in this module.
//!
//! The [`GbMetricsRecorder`] trait is the seam through which higher layers
//! (application services, event sinks) feed GB28181 activity into a metrics
//! aggregator without depending on any concrete runtime implementation. The
//! aggregator itself (implementing this trait and the Prometheus exporter)
//! lives in the Tokio runtime crate.

/// Bounded category for a dispatched GB28181 command.
///
/// Concrete device/channel identifiers are never encoded here; the variant set
/// is fixed so the `method` label cardinality is constant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GbCommandMethod {
    /// PTZ movement / zoom / focus / iris control.
    Ptz,
    /// Non-PTZ device control (reboot, guard, record, alarm reset, ...).
    DeviceControl,
    /// Device configuration writes.
    DeviceConfig,
    /// Catalog / device info / device status queries.
    Query,
    /// Recording index (RecordInfo) queries.
    RecordInfo,
    /// Voice broadcast / talk setup.
    Broadcast,
    /// Any other or unclassified command method.
    Other,
}

impl GbCommandMethod {
    /// All variants, used to pre-allocate bounded metric series.
    pub const ALL: [GbCommandMethod; 7] = [
        GbCommandMethod::Ptz,
        GbCommandMethod::DeviceControl,
        GbCommandMethod::DeviceConfig,
        GbCommandMethod::Query,
        GbCommandMethod::RecordInfo,
        GbCommandMethod::Broadcast,
        GbCommandMethod::Other,
    ];

    /// Stable, low-cardinality label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            GbCommandMethod::Ptz => "ptz",
            GbCommandMethod::DeviceControl => "device_control",
            GbCommandMethod::DeviceConfig => "device_config",
            GbCommandMethod::Query => "query",
            GbCommandMethod::RecordInfo => "record_info",
            GbCommandMethod::Broadcast => "broadcast",
            GbCommandMethod::Other => "other",
        }
    }

    /// Dense index into a fixed-size series array.
    pub const fn index(self) -> usize {
        match self {
            GbCommandMethod::Ptz => 0,
            GbCommandMethod::DeviceControl => 1,
            GbCommandMethod::DeviceConfig => 2,
            GbCommandMethod::Query => 3,
            GbCommandMethod::RecordInfo => 4,
            GbCommandMethod::Broadcast => 5,
            GbCommandMethod::Other => 6,
        }
    }
}

/// Bounded outcome for a dispatched GB28181 command.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GbCommandOutcome {
    /// Command was dispatched to the owner/device but no result is known yet.
    Dispatched,
    /// Command completed successfully.
    Succeeded,
    /// Command failed with a classified error.
    Failed,
    /// The command may or may not have taken effect on the device.
    Unknown,
    /// Command was cancelled before completion.
    Cancelled,
}

impl GbCommandOutcome {
    /// All variants, used to pre-allocate bounded metric series.
    pub const ALL: [GbCommandOutcome; 5] = [
        GbCommandOutcome::Dispatched,
        GbCommandOutcome::Succeeded,
        GbCommandOutcome::Failed,
        GbCommandOutcome::Unknown,
        GbCommandOutcome::Cancelled,
    ];

    /// Stable, low-cardinality label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            GbCommandOutcome::Dispatched => "dispatched",
            GbCommandOutcome::Succeeded => "succeeded",
            GbCommandOutcome::Failed => "failed",
            GbCommandOutcome::Unknown => "unknown",
            GbCommandOutcome::Cancelled => "cancelled",
        }
    }

    /// Dense index into a fixed-size series array.
    pub const fn index(self) -> usize {
        match self {
            GbCommandOutcome::Dispatched => 0,
            GbCommandOutcome::Succeeded => 1,
            GbCommandOutcome::Failed => 2,
            GbCommandOutcome::Unknown => 3,
            GbCommandOutcome::Cancelled => 4,
        }
    }
}

/// Bounded presence category for the device-count gauge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GbDevicePresence {
    /// Device is currently online.
    Online,
    /// Device is currently offline.
    Offline,
}

impl GbDevicePresence {
    /// All variants, used to pre-allocate bounded metric series.
    pub const ALL: [GbDevicePresence; 2] = [GbDevicePresence::Online, GbDevicePresence::Offline];

    /// Stable, low-cardinality label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            GbDevicePresence::Online => "online",
            GbDevicePresence::Offline => "offline",
        }
    }

    /// Dense index into a fixed-size series array.
    pub const fn index(self) -> usize {
        match self {
            GbDevicePresence::Online => 0,
            GbDevicePresence::Offline => 1,
        }
    }
}

/// Bounded lifecycle state for the media-session gauge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GbMediaSessionState {
    /// Session requested; a binding is being negotiated.
    Pending,
    /// Session is active with a live binding.
    Active,
    /// Session is being torn down.
    Stopping,
    /// Session reached a terminal state.
    Terminated,
}

impl GbMediaSessionState {
    /// All variants, used to pre-allocate bounded metric series.
    pub const ALL: [GbMediaSessionState; 4] = [
        GbMediaSessionState::Pending,
        GbMediaSessionState::Active,
        GbMediaSessionState::Stopping,
        GbMediaSessionState::Terminated,
    ];

    /// Stable, low-cardinality label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            GbMediaSessionState::Pending => "pending",
            GbMediaSessionState::Active => "active",
            GbMediaSessionState::Stopping => "stopping",
            GbMediaSessionState::Terminated => "terminated",
        }
    }

    /// Dense index into a fixed-size series array.
    pub const fn index(self) -> usize {
        match self {
            GbMediaSessionState::Pending => 0,
            GbMediaSessionState::Active => 1,
            GbMediaSessionState::Stopping => 2,
            GbMediaSessionState::Terminated => 3,
        }
    }
}

/// Port through which higher layers report GB28181 application activity to a
/// metrics aggregator.
///
/// Only bounded categories are accepted; identifiers are supplied out of band
/// (traces/logs) and never reach the metrics label space. Implementations must
/// be cheap and lock-free on the hot path.
pub trait GbMetricsRecorder: Send + Sync {
    /// Records a single dispatched command with its bounded method/outcome.
    fn record_command(&self, method: GbCommandMethod, outcome: GbCommandOutcome);

    /// Records one received GB28181 catalog fragment.
    fn record_catalog_fragment(&self);

    /// Sets the current number of in-flight application operations.
    fn set_active_operations(&self, count: u64);

    /// Sets the current device count for a bounded presence category.
    fn set_device_gauge(&self, presence: GbDevicePresence, count: u64);

    /// Sets the current media-session count for a bounded lifecycle state.
    fn set_media_session_gauge(&self, state: GbMediaSessionState, count: u64);

    /// Sets the current number of established cascade links.
    fn set_cascade_link_total(&self, count: u64);
}

/// A recorder that discards every sample.
///
/// Useful as a default when metrics wiring is optional.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopGbMetricsRecorder;

impl GbMetricsRecorder for NoopGbMetricsRecorder {
    fn record_command(&self, _method: GbCommandMethod, _outcome: GbCommandOutcome) {}
    fn record_catalog_fragment(&self) {}
    fn set_active_operations(&self, _count: u64) {}
    fn set_device_gauge(&self, _presence: GbDevicePresence, _count: u64) {}
    fn set_media_session_gauge(&self, _state: GbMediaSessionState, _count: u64) {}
    fn set_cascade_link_total(&self, _count: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_method_indices_are_dense_and_unique() {
        for (i, method) in GbCommandMethod::ALL.iter().enumerate() {
            assert_eq!(method.index(), i);
            assert!(!method.as_str().is_empty());
        }
    }

    #[test]
    fn command_outcome_indices_are_dense_and_unique() {
        for (i, outcome) in GbCommandOutcome::ALL.iter().enumerate() {
            assert_eq!(outcome.index(), i);
        }
    }

    #[test]
    fn presence_and_state_indices_are_dense() {
        for (i, presence) in GbDevicePresence::ALL.iter().enumerate() {
            assert_eq!(presence.index(), i);
        }
        for (i, state) in GbMediaSessionState::ALL.iter().enumerate() {
            assert_eq!(state.index(), i);
        }
    }

    #[test]
    fn noop_recorder_is_inert() {
        let recorder = NoopGbMetricsRecorder;
        recorder.record_command(GbCommandMethod::Ptz, GbCommandOutcome::Succeeded);
        recorder.record_catalog_fragment();
        recorder.set_active_operations(3);
        recorder.set_device_gauge(GbDevicePresence::Online, 10);
        recorder.set_media_session_gauge(GbMediaSessionState::Active, 2);
        recorder.set_cascade_link_total(1);
    }
}

//! Domain command wrapper for GB28181 outbound routing.

use crate::types::DeviceId;
use cheetah_domain::Command;

/// A domain command targeted at a GB28181 device or channel.
#[derive(Clone, Debug)]
pub struct Gb28181Command {
    /// Original domain command.
    pub command: Command,
    /// Resolved external GB device identifier.
    pub device_external_id: DeviceId,
    /// Resolved external GB channel identifier, if the command is scoped to a
    /// channel and the channel carries its own GB ID.
    pub channel_external_id: Option<DeviceId>,
    /// Identifier of the configured listener that the device registered on.
    pub listener_id: String,
}

impl Gb28181Command {
    /// Creates a new GB28181-targeted command.
    pub fn new(
        command: Command,
        device_external_id: DeviceId,
        channel_external_id: Option<DeviceId>,
        listener_id: String,
    ) -> Self {
        Self {
            command,
            device_external_id,
            channel_external_id,
            listener_id,
        }
    }
}

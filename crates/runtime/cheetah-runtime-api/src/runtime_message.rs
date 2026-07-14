//! Runtime messages that can be sent to a shard worker.

use cheetah_domain::Command;
use cheetah_signal_types::OwnerEpoch;

use crate::{DeviceKey, TimerId};

/// A message processed by a shard worker.
#[derive(Clone, Debug)]
pub enum RuntimeMessage {
    /// A protocol-specific event payload.
    ProtocolEvent {
        /// Target device key.
        device_key: DeviceKey,
        /// Opaque payload bytes.
        payload: Vec<u8>,
    },

    /// An immutable command to be executed.
    Command {
        /// Target device key.
        device_key: DeviceKey,
        /// Immutable command.
        command: Box<Command>,
    },

    /// A timer that has fired.
    Timer {
        /// Target device key.
        device_key: DeviceKey,
        /// Timer identifier.
        timer_id: TimerId,
        /// Caller-provided timer kind.
        kind: String,
    },

    /// Ownership of the device has changed.
    OwnershipChanged {
        /// Target device key.
        device_key: DeviceKey,
        /// New owner epoch.
        owner_epoch: OwnerEpoch,
    },

    /// Request to shut down a shard.
    Shutdown,
}

impl RuntimeMessage {
    /// Returns the target device key, if the message has one.
    pub fn device_key(&self) -> Option<DeviceKey> {
        match self {
            Self::ProtocolEvent { device_key, .. }
            | Self::Command { device_key, .. }
            | Self::Timer { device_key, .. }
            | Self::OwnershipChanged { device_key, .. } => Some(*device_key),
            Self::Shutdown => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_key_extracted_from_protocol_event() {
        let tenant_id = cheetah_signal_types::TenantId::generate();
        let device_id = cheetah_signal_types::DeviceId::generate();
        let key = DeviceKey::new(tenant_id, device_id);
        let msg = RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![1, 2, 3],
        };
        assert_eq!(msg.device_key(), Some(key));
    }

    #[test]
    fn shutdown_has_no_device_key() {
        let msg = RuntimeMessage::Shutdown;
        assert_eq!(msg.device_key(), None);
    }
}

//! Strongly typed identifiers for Cheetah Signaling.
//!
//! Internal IDs are based on UUIDv7 and are validated at construction time.
//! External protocol identities are represented by [`ProtocolIdentity`].

use crate::error::{Result, SignalError, SignalErrorKind};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

const ID_STRING_LEN: usize = 36;
const PROTOCOL_IDENTITY_MAX_BYTES: usize = 256;

/// Macro generating a UUIDv7 based newtype identifier.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident, $doc:expr) => {
        $(#[$meta])*
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        #[repr(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new identifier using UUIDv7.
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }

            /// Constructs an identifier from a validated UUID.
            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the underlying UUID.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }

            /// Returns the bytes of the UUID.
            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }

            /// Constructs from a 16 byte array.
            pub fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(Uuid::from_bytes(bytes))
            }

            /// Encodes the identifier as a compact byte vector.
            pub fn to_vec(&self) -> Vec<u8> {
                self.0.as_bytes().to_vec()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = SignalError;

            fn from_str(s: &str) -> Result<Self> {
                if s.len() != ID_STRING_LEN {
                    return Err(SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        format!("{} string must be a UUID", stringify!($name)),
                    ));
                }
                let uuid = Uuid::parse_str(s).map_err(|e| {
                    SignalError::new(
                        SignalErrorKind::InvalidArgument,
                        format!("invalid {}: {e}", stringify!($name)),
                    )
                    .with_source(e)
                })?;
                Ok(Self::from_uuid(uuid))
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self::from_uuid(value)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::from_uuid(Uuid::nil())
            }
        }
    };
}

uuid_id! {
    /// Identifier for a tenant.
    TenantId,
    "Tenant identifier."
}

uuid_id! {
    /// Identifier for a device.
    DeviceId,
    "Device identifier."
}

uuid_id! {
    /// Identifier for an endpoint within a device.
    EndpointId,
    "Endpoint identifier."
}

uuid_id! {
    /// Identifier for a channel within a device.
    ChannelId,
    "Channel identifier."
}

uuid_id! {
    /// Identifier for a protocol session.
    ProtocolSessionId,
    "Protocol session identifier."
}

uuid_id! {
    /// Identifier for a media session.
    MediaSessionId,
    "Media session identifier."
}

uuid_id! {
    /// Identifier for a media binding.
    MediaBindingId,
    "Media binding identifier."
}

uuid_id! {
    /// Identifier for an operation.
    OperationId,
    "Operation identifier."
}

uuid_id! {
    /// Identifier for a node in the cluster.
    NodeId,
    "Node identifier."
}

uuid_id! {
    /// Identifier for a plugin instance.
    PluginId,
    "Plugin identifier."
}

uuid_id! {
    /// Identifier for a domain event.
    EventId,
    "Event identifier."
}

uuid_id! {
    /// Identifier for a message.
    MessageId,
    "Message identifier."
}

uuid_id! {
    /// Identifier for correlating requests across services.
    CorrelationId,
    "Correlation identifier."
}

/// An identity used by an external protocol before it has been mapped to an internal UUID.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct ProtocolIdentity(String);

impl ProtocolIdentity {
    /// Creates a new protocol identity after validating length and character set.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "protocol identity must not be empty",
            ));
        }
        if value.len() > PROTOCOL_IDENTITY_MAX_BYTES {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("protocol identity exceeds {PROTOCOL_IDENTITY_MAX_BYTES} bytes"),
            ));
        }
        if value.contains('\0') {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "protocol identity must not contain NUL",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the identity as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the owned string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ProtocolIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ProtocolIdentity {
    type Err = SignalError;

    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

impl AsRef<str> for ProtocolIdentity {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProtocolIdentity {
    type Error = SignalError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> serde::Deserialize<'de> for ProtocolIdentity {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ProtocolIdentityVisitor;

        impl serde::de::Visitor<'_> for ProtocolIdentityVisitor {
            type Value = ProtocolIdentity;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "a non-empty protocol identity string")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                ProtocolIdentity::new(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(ProtocolIdentityVisitor)
    }
}

/// Optimistic concurrency revision.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Revision(pub u64);

/// Fencing epoch for ownership across nodes.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct OwnerEpoch(pub u64);

/// Instance epoch of a media node instance.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct MediaNodeInstanceEpoch(pub u64);

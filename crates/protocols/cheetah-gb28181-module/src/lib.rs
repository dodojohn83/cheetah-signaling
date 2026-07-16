//! GB28181 southbound device-access protocol module.
//!
//! This crate maps SIP/XML wire messages to protocol-level outputs. It is
//! intentionally Sans-I/O: network transport is handled by
//! `cheetah-gb28181-driver-tokio` and persistence/application orchestration by
//! the application layer above.
//!
//! # Architecture
//!
//! - `config` – per-realm GB28181 configuration, authentication policy and
//!   XML limits.
//! - `device_id` – validation and extraction of GB28181 device/channel IDs.
//! - `xml` – bounded XML codec for MANSCDP messages.
//! - `output` – protocol outputs produced by the module (registrations,
//!   heartbeats, catalogs, etc.).
//! - `module` – core `Gb28181Module` state machine.
//! - `actor` – `DeviceActor` integration with `cheetah-runtime-api`.

#![warn(missing_docs)]

pub mod actor;
pub mod config;
pub mod device_id;
pub mod module;
pub mod output;
pub mod xml;

pub use actor::Gb28181Actor;
pub use config::{
    AuthPolicy, CharsetPolicy, CompatibilityProfile, Gb28181Config, Gb28181ConfigBuilder,
    InMemoryPasswordLookup, MatchConditions, PasswordLookup, WorkaroundBehavior, XmlLimits,
};
pub use error::Gb28181ModuleError;
pub use module::{Gb28181Input, Gb28181Module};
pub use output::{
    Gb28181Alarm, Gb28181Catalog, Gb28181CatalogItem, Gb28181CommandResult, Gb28181DeviceInfo,
    Gb28181DeviceStatus, Gb28181Heartbeat, Gb28181MobilePosition, Gb28181Output, Gb28181RecordInfo,
    Gb28181RecordItem, Gb28181Refresh, Gb28181Register,
};

mod error;

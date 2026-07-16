//! ONVIF protocol module.
//!
//! The full business logic is currently in the `devin/phase-17-onvif-services`
//! branch. This crate exposes a minimal built-in driver adapter so the plugin
//! host can register and lifecycle-manage the ONVIF factory.

#![warn(missing_docs)]

pub mod driver;

pub use driver::{OnvifDriverFactory, OnvifProtocolDriver};

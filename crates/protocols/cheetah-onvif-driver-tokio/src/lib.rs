//! Tokio driver for ONVIF: WS-Discovery over UDP and SOAP 1.2 over HTTP.
//!
//! Business mapping lives in `cheetah-onvif-module`. This crate only performs
//! network I/O with deadlines, body limits and SSRF policy enforcement.
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod auth;
pub mod capability_cache;
pub(crate) mod commands;
pub mod config;
pub mod discovery;
pub(crate) mod driver;
pub mod error;
pub(crate) mod events;
pub mod protocol_driver;
pub mod soap_client;
pub(crate) mod util;

pub use auth::{DeviceCredentials, inject_username_token};
pub use config::DriverConfig;
pub use discovery::{DiscoveryResult, probe_once, validate_endpoint};
pub use driver::OnvifHttpDriver;
pub use error::{DriverError, DriverResult};
pub use protocol_driver::{OnvifTokioDriverFactory, OnvifTokioProtocolDriver};
pub use soap_client::SoapClient;

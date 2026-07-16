//! Sans-I/O ONVIF core: WS-Discovery message model, SOAP 1.2 envelope helpers,
//! and WS-Security `UsernameToken` construction.
//!
//! This crate contains no network I/O. `cheetah-onvif-driver-tokio` will use the
//! builders/parsers and drive the actual multicast and HTTP sockets.

pub mod discovery;
pub mod error;
pub mod security;
pub mod soap;

pub use error::{OnvifError, OnvifResult};

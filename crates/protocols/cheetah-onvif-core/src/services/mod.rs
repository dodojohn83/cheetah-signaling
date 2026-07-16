//! Typed ONVIF service request builders and response parsers.
//!
//! These helpers produce raw SOAP body fragments and parse responses without
//! performing HTTP I/O, which remains in the driver crate.

pub mod system_date_time;

pub use system_date_time::{
    DateTime, SystemDateAndTime, build_get_system_date_and_time,
    parse_get_system_date_and_time_response,
};

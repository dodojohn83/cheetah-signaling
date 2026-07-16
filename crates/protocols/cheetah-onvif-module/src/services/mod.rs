//! ONVIF service request builders and response parsers.

pub mod device;

pub use device::{
    SystemDateAndTime, get_capabilities_request, get_device_information_request,
    get_hostname_request, get_network_interfaces_request, get_services_request,
    get_system_date_and_time_request, parse_get_capabilities_response,
    parse_get_device_information_response, parse_get_services_response,
};

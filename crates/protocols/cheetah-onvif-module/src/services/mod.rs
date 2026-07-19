//! ONVIF service request builders and response parsers.

pub mod device;
pub mod events;
pub mod imaging;
pub mod media;
pub(crate) mod parse;
pub mod ptz;

pub use device::{
    SystemDateAndTime, get_capabilities_request, get_device_information_request,
    get_hostname_request, get_network_interfaces_request, get_services_request,
    get_system_date_and_time_request, parse_get_capabilities_response,
    parse_get_device_information_response, parse_get_services_response,
    parse_get_system_date_and_time_response,
};
pub use events::{
    OnvifNotification, PullPointSubscription, create_pull_point_subscription_request,
    normalize_topic, parse_create_pull_point_response, parse_pull_messages_response,
    pull_messages_request, renew_request, unsubscribe_request,
};
pub use imaging::{
    ImagingSettings, get_imaging_options_request, get_imaging_settings_request,
    parse_get_imaging_settings_response, set_imaging_settings_action,
    set_imaging_settings_request,
};
pub use media::{
    MediaDialect, MediaProfile, SnapshotUri, StreamUri, get_profiles_request,
    get_snapshot_uri_request, get_stream_uri_request_media1, get_stream_uri_request_media2,
    parse_get_profiles_response, parse_get_snapshot_uri_response, parse_get_stream_uri_response,
    redact_uri_userinfo, validate_media_uri,
};
pub use ptz::{
    PtzPreset, PtzVector, PtzVelocity, absolute_move_request, clip_unit, continuous_move_request,
    get_presets_request, goto_preset_request, parse_get_presets_response, relative_move_request,
    stop_request,
};

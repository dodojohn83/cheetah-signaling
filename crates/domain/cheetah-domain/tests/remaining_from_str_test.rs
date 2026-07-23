//! Regression tests for the remaining domain `FromStr` implementations that were
//! not covered by PR #340 and now avoid allocation + bound error messages.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;

use cheetah_domain::{
    BroadcastAddressSource, CompatibilityCapability, PlatformDirection, PresenceState, SipTransport,
};

#[test]
fn sip_transport_from_str_is_case_insensitive_and_bounded() {
    assert_eq!(SipTransport::from_str("UDP").unwrap(), SipTransport::Udp);
    assert_eq!(SipTransport::from_str("tcp").unwrap(), SipTransport::Tcp);
    let err = SipTransport::from_str("x".repeat(128).as_str()).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.len() <= 128,
        "error message should be bounded, got {msg}"
    );
}

#[test]
fn presence_state_from_str_is_case_insensitive() {
    assert_eq!(
        PresenceState::from_str("ONLINE").unwrap(),
        PresenceState::Online
    );
    assert_eq!(
        PresenceState::from_str("Offline").unwrap(),
        PresenceState::Offline
    );
    assert_eq!(
        PresenceState::from_str("Unknown").unwrap(),
        PresenceState::Unknown
    );
    assert!(PresenceState::from_str("away").is_err());
}

#[test]
fn platform_direction_from_str_is_case_insensitive() {
    assert_eq!(
        PlatformDirection::from_str("UPSTREAM").unwrap(),
        PlatformDirection::Upstream
    );
    assert_eq!(
        PlatformDirection::from_str("Downstream").unwrap(),
        PlatformDirection::Downstream
    );
    let err = PlatformDirection::from_str("x".repeat(128).as_str()).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.len() <= 128,
        "error message should be bounded, got {msg}"
    );
}

#[test]
fn broadcast_address_source_from_str_normalizes_hyphens() {
    assert_eq!(
        BroadcastAddressSource::from_str("media-node").unwrap(),
        BroadcastAddressSource::MediaNode
    );
    assert_eq!(
        BroadcastAddressSource::from_str("Media_Node").unwrap(),
        BroadcastAddressSource::MediaNode
    );
    assert_eq!(
        BroadcastAddressSource::from_str("SIGNALING-HOST").unwrap(),
        BroadcastAddressSource::SignalingHost
    );
    assert!(BroadcastAddressSource::from_str("invalid").is_err());
}

#[test]
fn compatibility_capability_from_str_normalizes_hyphens() {
    assert_eq!(
        CompatibilityCapability::from_str("mime-alias").unwrap(),
        CompatibilityCapability::MimeAlias
    );
    assert_eq!(
        CompatibilityCapability::from_str("contact-rport-route").unwrap(),
        CompatibilityCapability::ContactRportRoute
    );
    assert_eq!(
        CompatibilityCapability::from_str("Device_Per_Password").unwrap(),
        CompatibilityCapability::DevicePerPassword
    );
    let err = CompatibilityCapability::from_str("x".repeat(256).as_str()).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.len() <= 128,
        "error message should be bounded, got {msg}"
    );
}

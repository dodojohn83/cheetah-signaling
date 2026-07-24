#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(missing_docs)]

use std::str::FromStr;

use cheetah_domain::{
    ChannelKind, ChannelStatus, DeliveryStatus, DeviceKind, DeviceLifecycle, MediaPurpose,
    MediaSessionState, OperationStatus, Protocol,
};

const OVERSIZED: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn assert_rejects<T: FromStr<Err = cheetah_domain::DomainError> + std::fmt::Debug>(
    input: &str,
    expected_prefix: &str,
) {
    let err = T::from_str(input).unwrap_err();
    match err {
        cheetah_domain::DomainError::InvalidArgument { message } => {
            assert!(
                message.len() <= 128,
                "error message should be bounded, got {} bytes",
                message.len()
            );
            assert!(message.starts_with(expected_prefix), "{message}");
        }
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn device_kind_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(DeviceKind::from_str("camera").unwrap(), DeviceKind::Camera);
    assert_eq!(DeviceKind::from_str("NVR").unwrap(), DeviceKind::Nvr);
    assert_eq!(DeviceKind::from_str("IoT").unwrap(), DeviceKind::Iot);
    assert_rejects::<DeviceKind>("unknown", "unknown device kind:");
    assert_rejects::<DeviceKind>(OVERSIZED, "unknown device kind:");
}

#[test]
fn protocol_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(Protocol::from_str("gb28181").unwrap(), Protocol::Gb28181);
    assert_eq!(Protocol::from_str("ONVIF").unwrap(), Protocol::Onvif);
    assert_rejects::<Protocol>("ftp", "unknown protocol:");
    assert_rejects::<Protocol>(OVERSIZED, "unknown protocol:");
}

#[test]
fn device_lifecycle_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(
        DeviceLifecycle::from_str("retired").unwrap(),
        DeviceLifecycle::Retired
    );
    assert_eq!(
        DeviceLifecycle::from_str("Active").unwrap(),
        DeviceLifecycle::Active
    );
    assert_rejects::<DeviceLifecycle>("deleted", "unknown lifecycle:");
    assert_rejects::<DeviceLifecycle>(OVERSIZED, "unknown lifecycle:");
}

#[test]
fn channel_kind_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(ChannelKind::from_str("video").unwrap(), ChannelKind::Video);
    assert_eq!(ChannelKind::from_str("PTZ").unwrap(), ChannelKind::Ptz);
    assert_eq!(ChannelKind::from_str("Io").unwrap(), ChannelKind::Io);
    assert_rejects::<ChannelKind>("foo", "unknown channel kind:");
    assert_rejects::<ChannelKind>(OVERSIZED, "unknown channel kind:");
}

#[test]
fn channel_status_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(
        ChannelStatus::from_str("online").unwrap(),
        ChannelStatus::Online
    );
    assert_eq!(
        ChannelStatus::from_str("FAULT").unwrap(),
        ChannelStatus::Fault
    );
    assert_rejects::<ChannelStatus>("unknown", "unknown channel status:");
    assert_rejects::<ChannelStatus>(OVERSIZED, "unknown channel status:");
}

#[test]
fn media_session_state_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(
        MediaSessionState::from_str("active").unwrap(),
        MediaSessionState::Active
    );
    assert_eq!(
        MediaSessionState::from_str("FAILED").unwrap(),
        MediaSessionState::Failed
    );
    assert_rejects::<MediaSessionState>("idle", "unknown media session state:");
    assert_rejects::<MediaSessionState>(OVERSIZED, "unknown media session state:");
}

#[test]
fn media_purpose_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(
        MediaPurpose::from_str("playback").unwrap(),
        MediaPurpose::Playback
    );
    assert_eq!(
        MediaPurpose::from_str("Broadcast").unwrap(),
        MediaPurpose::Broadcast
    );
    assert_rejects::<MediaPurpose>("idle", "unknown media purpose:");
    assert_rejects::<MediaPurpose>(OVERSIZED, "unknown media purpose:");
}

#[test]
fn operation_status_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(
        OperationStatus::from_str("cancelled").unwrap(),
        OperationStatus::Cancelled
    );
    assert_eq!(
        OperationStatus::from_str("Running").unwrap(),
        OperationStatus::Running
    );
    assert_rejects::<OperationStatus>("done", "unknown operation status:");
    assert_rejects::<OperationStatus>(OVERSIZED, "unknown operation status:");
}

#[test]
fn delivery_status_parses_case_insensitive_and_rejects_invalid() {
    assert_eq!(
        DeliveryStatus::from_str("dead_letter").unwrap(),
        DeliveryStatus::DeadLetter
    );
    assert_eq!(
        DeliveryStatus::from_str("In_Progress").unwrap(),
        DeliveryStatus::InProgress
    );
    assert_rejects::<DeliveryStatus>("queued", "unknown delivery status:");
    assert_rejects::<DeliveryStatus>(OVERSIZED, "unknown delivery status:");
}

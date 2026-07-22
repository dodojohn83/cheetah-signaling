//! Tests for the typed GB28181 media command mapper and golden SIP/SDP output.

use super::*;
use crate::media::mapper::{
    GbMediaEndpoint, GbMediaPurpose, GbRecordWindow, GbSipRouting, GbStartRequest, map_control,
    map_start,
};
use cheetah_domain::{MediaControl, MediaPurpose};
use cheetah_gb28181_core::encode_message;

fn routing() -> GbSipRouting {
    GbSipRouting {
        target: SipUri::parse("sip:34020000001320000001@192.168.1.20:5060").unwrap(),
        device_id: DeviceId::new("34020000001320000001").unwrap(),
        call_id: "call-1".to_string(),
        local_tag: "tag-local".to_string(),
        cseq: 1,
        branch: "z9hG4bK1234".to_string(),
        subject_session: "0200000000".to_string(),
    }
}

fn video_endpoint(transport: MediaTransport) -> GbMediaEndpoint {
    GbMediaEndpoint {
        media_address: "192.168.1.100".to_string(),
        media_port: 5000,
        ssrc: Some("0200000000".to_string()),
        transport,
    }
}

fn window() -> GbRecordWindow {
    GbRecordWindow {
        start_time: "1704067200".to_string(),
        end_time: "1704153600".to_string(),
    }
}

fn base_request(purpose: GbMediaPurpose) -> GbStartRequest {
    GbStartRequest {
        media_session_id: MediaSessionId::generate(),
        channel_id: ChannelId::generate(),
        purpose,
        routing: routing(),
        endpoint: video_endpoint(MediaTransport::Udp),
        window: None,
        download_speed: None,
        codec: None,
    }
}

/// Compares `actual` against a committed golden fixture. Set `UPDATE_GOLDEN=1`
/// to (re)generate the fixtures under `src/media/tests/golden/`.
fn assert_golden(name: &str, actual: &[u8]) {
    let path = format!(
        "{}/src/media/tests/golden/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        let parent = std::path::Path::new(&path).parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read(&path).unwrap_or_else(|e| panic!("read golden {path}: {e}"));
    assert_eq!(
        String::from_utf8_lossy(actual),
        String::from_utf8_lossy(&expected),
        "golden mismatch for {name}"
    );
}

fn first_send(outputs: Vec<MediaOutput>) -> SipMessage {
    match outputs.into_iter().next().expect("at least one output") {
        MediaOutput::SendMessage(msg) => msg,
        MediaOutput::EmitEvent(_) => panic!("expected SendMessage"),
    }
}

// ---- Start mapping + golden INVITE fixtures --------------------------------

#[test]
fn map_live_produces_golden_invite() {
    let mut req = base_request(GbMediaPurpose::Live);
    req.endpoint = video_endpoint(MediaTransport::TcpPassive);
    let cmd = map_start(req).unwrap();
    let mut media = Gb28181Media::new(config());
    let invite = first_send(media.process(MediaInput::Command(cmd)).unwrap());
    assert_golden("live_invite.sip", &encode_message(&invite));
}

#[test]
fn map_playback_produces_golden_invite() {
    let mut req = base_request(GbMediaPurpose::Playback);
    req.window = Some(window());
    let cmd = map_start(req).unwrap();
    let mut media = Gb28181Media::new(config());
    let invite = first_send(media.process(MediaInput::Command(cmd)).unwrap());
    assert_golden("playback_invite.sip", &encode_message(&invite));
}

#[test]
fn map_download_produces_golden_invite() {
    let mut req = base_request(GbMediaPurpose::Download);
    req.window = Some(window());
    req.download_speed = Some(4);
    let cmd = map_start(req).unwrap();
    let mut media = Gb28181Media::new(config());
    let invite = first_send(media.process(MediaInput::Command(cmd)).unwrap());
    assert_golden("download_invite.sip", &encode_message(&invite));
}

#[test]
fn map_talk_produces_golden_invite() {
    let mut req = base_request(GbMediaPurpose::Talk);
    req.endpoint.ssrc = None;
    req.codec = Some("G.711A".to_string());
    let cmd = map_start(req).unwrap();
    let mut media = Gb28181Media::new(config());
    let invite = first_send(media.process(MediaInput::Command(cmd)).unwrap());
    assert_golden("talk_invite.sip", &encode_message(&invite));
}

// ---- Control mapping + golden MANSRTSP INFO fixtures -----------------------

/// Drives a playback session to the Active state so in-dialog INFO can be sent.
fn active_playback() -> (Gb28181Media, MediaSessionId) {
    let sid = MediaSessionId::generate();
    let mut req = base_request(GbMediaPurpose::Playback);
    req.media_session_id = sid;
    req.window = Some(window());
    let cmd = map_start(req).unwrap();
    let mut media = Gb28181Media::new(config());
    media.process(MediaInput::Command(cmd)).unwrap();
    media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();
    (media, sid)
}

fn info_golden(name: &str, control: MediaControl) {
    let (mut media, sid) = active_playback();
    let cmd = map_control(sid, &control).unwrap();
    let info = first_send(media.process(MediaInput::Command(cmd)).unwrap());
    assert_golden(name, &encode_message(&info));
}

#[test]
fn map_control_play_produces_golden_info() {
    info_golden("control_play.sip", MediaControl::Play);
}

#[test]
fn map_control_pause_produces_golden_info() {
    info_golden("control_pause.sip", MediaControl::Pause);
}

#[test]
fn map_control_seek_produces_golden_info() {
    info_golden("control_seek.sip", MediaControl::Seek { offset_ms: 30_000 });
}

#[test]
fn map_control_scale_produces_golden_info() {
    info_golden("control_scale.sip", MediaControl::Scale { value: 2.0 });
}

#[test]
fn map_control_stop_produces_teardown() {
    let cmd = map_control(MediaSessionId::generate(), &MediaControl::Stop).unwrap();
    match cmd {
        MediaCommand::ControlPlayback { action, .. } => {
            assert_eq!(action, PlaybackAction::Teardown);
        }
        _ => panic!("expected ControlPlayback"),
    }
}

// ---- Validation and unsupported-capability paths ---------------------------

#[test]
fn purpose_from_domain_maps_known_purposes() {
    assert_eq!(
        GbMediaPurpose::from_domain(MediaPurpose::Live).unwrap(),
        GbMediaPurpose::Live
    );
    assert_eq!(
        GbMediaPurpose::from_domain(MediaPurpose::Playback).unwrap(),
        GbMediaPurpose::Playback
    );
    assert_eq!(
        GbMediaPurpose::from_domain(MediaPurpose::Talk).unwrap(),
        GbMediaPurpose::Talk
    );
    assert!(matches!(
        GbMediaPurpose::from_domain(MediaPurpose::Unknown),
        Err(MediaError::Unsupported(_))
    ));
}

#[test]
fn live_requires_ssrc() {
    let mut req = base_request(GbMediaPurpose::Live);
    req.endpoint.ssrc = None;
    assert!(matches!(map_start(req), Err(MediaError::InvalidState(_))));
}

#[test]
fn recorded_purpose_requires_window() {
    let req = base_request(GbMediaPurpose::Playback);
    assert!(matches!(map_start(req), Err(MediaError::InvalidState(_))));
}

#[test]
fn live_rejects_recording_window() {
    let mut req = base_request(GbMediaPurpose::Live);
    req.window = Some(window());
    assert!(matches!(map_start(req), Err(MediaError::InvalidState(_))));
}

#[test]
fn download_rejects_zero_speed() {
    let mut req = base_request(GbMediaPurpose::Download);
    req.window = Some(window());
    req.download_speed = Some(0);
    assert!(matches!(map_start(req), Err(MediaError::InvalidState(_))));
}

#[test]
fn download_speed_on_non_download_is_rejected() {
    let mut req = base_request(GbMediaPurpose::Live);
    req.download_speed = Some(4);
    assert!(matches!(map_start(req), Err(MediaError::InvalidState(_))));
}

#[test]
fn talk_rejects_unsupported_codec() {
    let mut req = base_request(GbMediaPurpose::Talk);
    req.endpoint.ssrc = None;
    req.codec = Some("OPUS".to_string());
    assert!(matches!(map_start(req), Err(MediaError::Unsupported(_))));
}

#[test]
fn talk_rejects_video_ssrc() {
    let mut req = base_request(GbMediaPurpose::Talk);
    req.codec = Some("PCMA".to_string());
    assert!(matches!(map_start(req), Err(MediaError::InvalidState(_))));
}

#[test]
fn codec_on_non_talk_is_rejected() {
    let mut req = base_request(GbMediaPurpose::Live);
    req.codec = Some("G.711A".to_string());
    assert!(matches!(map_start(req), Err(MediaError::InvalidState(_))));
}

#[test]
fn seek_rejects_negative_offset() {
    let err = map_control(
        MediaSessionId::generate(),
        &MediaControl::Seek { offset_ms: -1 },
    );
    assert!(matches!(err, Err(MediaError::InvalidState(_))));
}

#[test]
fn scale_rejects_non_finite() {
    let err = map_control(
        MediaSessionId::generate(),
        &MediaControl::Scale {
            value: f64::INFINITY,
        },
    );
    assert!(matches!(err, Err(MediaError::InvalidState(_))));
}

#[test]
fn seek_maps_to_play_with_npt_range() {
    let cmd = map_control(
        MediaSessionId::generate(),
        &MediaControl::Seek { offset_ms: 30_000 },
    )
    .unwrap();
    match cmd {
        MediaCommand::ControlPlayback {
            action,
            scale,
            range,
            ..
        } => {
            assert_eq!(action, PlaybackAction::Play);
            assert!(scale.is_none());
            assert_eq!(range.as_deref(), Some("npt=30-"));
        }
        _ => panic!("expected ControlPlayback"),
    }
}

#[test]
fn scale_maps_to_play_with_scale() {
    let cmd = map_control(
        MediaSessionId::generate(),
        &MediaControl::Scale { value: 2.0 },
    )
    .unwrap();
    match cmd {
        MediaCommand::ControlPlayback {
            action,
            scale,
            range,
            ..
        } => {
            assert_eq!(action, PlaybackAction::Play);
            assert_eq!(scale, Some(2.0));
            assert!(range.is_none());
        }
        _ => panic!("expected ControlPlayback"),
    }
}

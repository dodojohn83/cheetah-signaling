//! Tests for compatibility-profile media negotiation overrides (GB4-COMP-003).

use super::*;
use cheetah_gb28181_core::{
    BroadcastAddressSource, BroadcastOverride, CompatibilityCapability, CompatibilityOverrides,
    CompatibilityProfile, SdpMediaOverride,
};

/// Builds a dialog-matching 200 OK whose single media description advertises the
/// given payload type plus any extra raw `a=` attribute lines.
fn ok_with_media(payload: &str, extra_attr_lines: &[&str]) -> SipMessage {
    let mut sdp = format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 0.0.0.0\r\n\
         s=Play\r\n\
         c=IN IP4 192.168.1.200\r\n\
         t=0 0\r\n\
         m=video 6000 TCP/RTP/AVP {payload}\r\n\
         a=setup:active\r\n\
         a=connection:new\r\n\
         a=rtpmap:{payload} PS/90000\r\n\
         a=y:0200000001"
    );
    for line in extra_attr_lines {
        sdp.push_str("\r\n");
        sdp.push_str(line);
    }
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bK1234"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 INVITE"));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5061>"),
    );
    headers.append(HeaderName::ContentType, HeaderValue::new("application/sdp"));
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(sdp.len().to_string()),
    );
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: sdp.into_bytes(),
    }
}

fn sdp_override_profile(payloads: &[&str], attrs: &[&str]) -> CompatibilityProfile {
    CompatibilityProfile {
        capabilities: vec![CompatibilityCapability::SdpMediaOverride],
        overrides: CompatibilityOverrides {
            sdp: Some(SdpMediaOverride {
                allowed_payload_types: payloads.iter().map(|s| s.to_string()).collect(),
                allowed_attribute_names: attrs.iter().map(|s| s.to_string()).collect(),
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn baseline_payload_accepted_without_override() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let outputs = media
        .process(MediaInput::Message(ok_with_media("96", &[])))
        .unwrap();
    assert!(matches!(
        outputs.last(),
        Some(MediaOutput::EmitEvent(
            Gb28181Event::MediaSessionStarted { .. }
        ))
    ));
}

#[test]
fn non_baseline_payload_rejected_by_strict_default() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let outputs = media
        .process(MediaInput::Message(ok_with_media("34", &[])))
        .unwrap();
    // ACK + BYE + failed event, and the session is torn down.
    assert_eq!(outputs.len(), 3);
    assert!(matches!(
        &outputs[2],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionFailed { .. })
    ));
    assert!(media.remove_session(sid).is_none());
}

#[test]
fn non_baseline_payload_accepted_with_profile_override() {
    let mut media = Gb28181Media::new(config_with_profile(sdp_override_profile(&["34"], &[])));
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let outputs = media
        .process(MediaInput::Message(ok_with_media("34", &[])))
        .unwrap();
    assert!(matches!(
        outputs.last(),
        Some(MediaOutput::EmitEvent(
            Gb28181Event::MediaSessionStarted { .. }
        ))
    ));
}

#[test]
fn unknown_attribute_rejected_by_strict_default() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let outputs = media
        .process(MediaInput::Message(ok_with_media(
            "96",
            &["a=vendorext:foo"],
        )))
        .unwrap();
    assert_eq!(outputs.len(), 3);
    assert!(matches!(
        &outputs[2],
        MediaOutput::EmitEvent(Gb28181Event::MediaSessionFailed { .. })
    ));
}

#[test]
fn unknown_attribute_accepted_with_profile_override() {
    let mut media = Gb28181Media::new(config_with_profile(sdp_override_profile(
        &[],
        &["vendorext"],
    )));
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    let outputs = media
        .process(MediaInput::Message(ok_with_media(
            "96",
            &["a=vendorext:foo"],
        )))
        .unwrap();
    assert!(matches!(
        outputs.last(),
        Some(MediaOutput::EmitEvent(
            Gb28181Event::MediaSessionStarted { .. }
        ))
    ));
}

#[test]
fn talk_offer_uses_media_node_address_by_default() {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    let outputs = media
        .process(MediaInput::Command(start_talk(sid, "PCMA")))
        .unwrap();
    let MediaOutput::SendMessage(SipMessage::Request { body, .. }) = &outputs[0] else {
        panic!("expected INVITE");
    };
    let sdp = String::from_utf8_lossy(body);
    assert!(sdp.contains("c=IN IP4 192.168.1.100"), "sdp: {sdp}");
}

#[test]
fn talk_offer_uses_signaling_host_with_broadcast_override() {
    let profile = CompatibilityProfile {
        capabilities: vec![CompatibilityCapability::Broadcast],
        overrides: CompatibilityOverrides {
            broadcast: Some(BroadcastOverride {
                address_source: BroadcastAddressSource::SignalingHost,
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut media = Gb28181Media::new(config_with_profile(profile));
    let sid = MediaSessionId::generate();
    let outputs = media
        .process(MediaInput::Command(start_talk(sid, "PCMA")))
        .unwrap();
    let MediaOutput::SendMessage(SipMessage::Request { body, .. }) = &outputs[0] else {
        panic!("expected INVITE");
    };
    let sdp = String::from_utf8_lossy(body);
    assert!(sdp.contains("c=IN IP4 192.168.1.10"), "sdp: {sdp}");
}

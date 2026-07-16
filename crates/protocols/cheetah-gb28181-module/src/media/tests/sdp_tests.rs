use cheetah_gb28181_core::sdp::{SdpOrigin, SdpSession, SdpTime};

#[test]
fn sdp_encoder_rejects_crlf_injection() {
    let session = SdpSession {
        version: "0".to_string(),
        origin: SdpOrigin {
            username: "-\r\ninject".to_string(),
            sess_id: "0".to_string(),
            sess_version: "0".to_string(),
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: "0.0.0.0".to_string(),
        },
        name: "Play".to_string(),
        times: vec![SdpTime {
            start: "0".to_string(),
            stop: "0".to_string(),
        }],
        ..Default::default()
    };
    let result = cheetah_gb28181_core::encode_sdp(&session);
    assert!(result.is_err());
}

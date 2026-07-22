//! Typed mapper from control-plane media intent to GB28181 [`MediaCommand`]s.
//!
//! The application-layer Saga owns the media *operation* lifecycle: it reserves
//! a media endpoint through `MediaPort`, resolves the signaling owner, and
//! decides the logical intent (start live/playback/download/talk, or control an
//! active playback). This module turns that already-fenced intent plus the
//! negotiated endpoint into the wire-facing [`MediaCommand`] consumed by the
//! GB28181 [`super::Gb28181Media`] state machine.
//!
//! The mapper is pure and Sans-I/O. It never allocates media ports, invents
//! SSRC/handle values, or contacts the media server; every physical value is
//! supplied by the caller from the `MediaPort` reservation. It validates the
//! structural invariants of each purpose (recording window presence, download
//! speed, audio codec) and returns a stable [`MediaError::Unsupported`] for
//! capabilities outside the v1 contract rather than silently degrading.

use super::control::PlaybackAction;
use super::{MediaCommand, MediaError, MediaTransport};
use crate::types::DeviceId;
use cheetah_domain::{MediaControl, MediaPurpose};
use cheetah_gb28181_core::SipUri;
use cheetah_signal_types::{ChannelId, MediaSessionId};

/// GB28181 media operation purpose.
///
/// Distinguishes `Download` from `Playback`, which the control-plane
/// [`MediaPurpose`] collapses into a single `Playback` value. The wire
/// behaviour differs (`s=Download` plus an `a=downloadspeed` attribute), so the
/// GB module keeps the distinction explicit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GbMediaPurpose {
    /// Real-time live preview.
    Live,
    /// Recorded video playback at (near) real time.
    Playback,
    /// Accelerated recorded video download.
    Download,
    /// Two-way audio talk.
    Talk,
}

impl GbMediaPurpose {
    /// Maps a control-plane [`MediaPurpose`] into a GB purpose.
    ///
    /// [`MediaPurpose::Playback`] maps to [`GbMediaPurpose::Playback`]; a
    /// download must be requested explicitly through [`GbMediaPurpose::Download`]
    /// because the control-plane purpose does not model it. [`MediaPurpose::Unknown`]
    /// and any future purpose return [`MediaError::Unsupported`].
    pub fn from_domain(purpose: MediaPurpose) -> Result<Self, MediaError> {
        match purpose {
            MediaPurpose::Live => Ok(Self::Live),
            MediaPurpose::Playback => Ok(Self::Playback),
            MediaPurpose::Talk => Ok(Self::Talk),
            other => Err(MediaError::Unsupported(format!(
                "media purpose {other:?} is not a GB28181 media operation"
            ))),
        }
    }

    /// Returns true when the purpose streams recorded video and therefore
    /// requires a recording window.
    fn is_recorded(self) -> bool {
        matches!(self, Self::Playback | Self::Download)
    }
}

/// SIP routing identity for a GB28181 media dialog.
///
/// Every field is generated and owner/epoch-fenced by the signaling owner
/// before the mapper runs; the mapper only forwards them into the SIP request.
#[derive(Clone, Debug)]
pub struct GbSipRouting {
    /// Remote device/channel AOR used in the `To` header and Request-URI.
    pub target: SipUri,
    /// GB28181 device identifier used in the `Subject` header.
    pub device_id: DeviceId,
    /// Dialog `Call-ID`.
    pub call_id: String,
    /// Local dialog tag.
    pub local_tag: String,
    /// INVITE `CSeq` number.
    pub cseq: u32,
    /// Top `Via` branch for the INVITE transaction.
    pub branch: String,
    /// GB28181 `Subject` session identifier (the `y=` SSRC session token).
    pub subject_session: String,
}

/// Negotiated media endpoint returned by `MediaPort`.
///
/// The media server owns the physical receiver/sender; the mapper copies these
/// values into the SDP offer without interpreting or binding them.
#[derive(Clone, Debug)]
pub struct GbMediaEndpoint {
    /// `c=`/origin address the device should send media to.
    pub media_address: String,
    /// `m=` port the device should send media to.
    pub media_port: u16,
    /// GB28181 `a=y:` SSRC for video sessions. Required for video, absent for
    /// audio talk.
    pub ssrc: Option<String>,
    /// Negotiated RTP transport.
    pub transport: MediaTransport,
}

/// Wall-clock recording window for playback/download.
///
/// The values are GB28181 `t=` seconds-since-epoch tokens. They are separate
/// from the monotonic operation deadline enforced by the Saga.
#[derive(Clone, Debug)]
pub struct GbRecordWindow {
    /// Inclusive start of the recording window (`t=` start).
    pub start_time: String,
    /// Exclusive end of the recording window (`t=` stop).
    pub end_time: String,
}

/// Typed, already-fenced request to start a GB28181 media session.
#[derive(Clone, Debug)]
pub struct GbStartRequest {
    /// Logical media session being started.
    pub media_session_id: MediaSessionId,
    /// Channel that sources the stream.
    pub channel_id: ChannelId,
    /// Media operation purpose.
    pub purpose: GbMediaPurpose,
    /// SIP routing identity.
    pub routing: GbSipRouting,
    /// Negotiated media endpoint.
    pub endpoint: GbMediaEndpoint,
    /// Recording window; required for playback/download, forbidden otherwise.
    pub window: Option<GbRecordWindow>,
    /// Download speed multiplier; only valid for [`GbMediaPurpose::Download`].
    pub download_speed: Option<u32>,
    /// Audio codec token; required for [`GbMediaPurpose::Talk`].
    pub codec: Option<String>,
}

/// Audio codecs supported for two-way talk.
///
/// Anything outside this set returns a stable [`MediaError::Unsupported`] rather
/// than transcoding or falling back to an undeclared codec.
const SUPPORTED_TALK_CODECS: &[&str] = &["G.711A", "PCMA", "G.711U", "PCMU"];

/// Maps a typed start request into a GB28181 [`MediaCommand`].
///
/// Validates the structural invariants of the requested purpose. Returns
/// [`MediaError::Unsupported`] for capabilities outside the v1 contract and
/// [`MediaError::InvalidState`] for internally inconsistent requests (a missing
/// recording window, a missing video SSRC, or a download speed on a
/// non-download purpose).
pub fn map_start(request: GbStartRequest) -> Result<MediaCommand, MediaError> {
    let GbStartRequest {
        media_session_id,
        channel_id,
        purpose,
        routing,
        endpoint,
        window,
        download_speed,
        codec,
    } = request;

    if !purpose.is_recorded() && window.is_some() {
        return Err(MediaError::InvalidState(format!(
            "{purpose:?} must not carry a recording window"
        )));
    }
    if download_speed.is_some() && purpose != GbMediaPurpose::Download {
        return Err(MediaError::InvalidState(format!(
            "{purpose:?} must not carry a download speed"
        )));
    }
    if codec.is_some() && purpose != GbMediaPurpose::Talk {
        return Err(MediaError::InvalidState(format!(
            "{purpose:?} must not carry an audio codec"
        )));
    }

    let GbSipRouting {
        target,
        device_id,
        call_id,
        local_tag,
        cseq,
        branch,
        subject_session,
    } = routing;
    let GbMediaEndpoint {
        media_address,
        media_port,
        ssrc,
        transport,
    } = endpoint;

    match purpose {
        GbMediaPurpose::Live => {
            let ssrc = require_video_ssrc(ssrc)?;
            Ok(MediaCommand::StartLive {
                media_session_id,
                channel_id,
                device_id,
                target,
                call_id,
                local_tag,
                cseq,
                branch,
                subject_session,
                media_address,
                media_port,
                ssrc,
                transport,
            })
        }
        GbMediaPurpose::Playback => {
            let ssrc = require_video_ssrc(ssrc)?;
            let window = require_window(window, purpose)?;
            Ok(MediaCommand::StartPlayback {
                media_session_id,
                channel_id,
                device_id,
                target,
                call_id,
                local_tag,
                cseq,
                branch,
                subject_session,
                media_address,
                media_port,
                ssrc,
                transport,
                start_time: window.start_time,
                end_time: window.end_time,
            })
        }
        GbMediaPurpose::Download => {
            let ssrc = require_video_ssrc(ssrc)?;
            let window = require_window(window, purpose)?;
            let download_speed = download_speed.ok_or_else(|| {
                MediaError::InvalidState("download requires a download speed".to_string())
            })?;
            if download_speed == 0 {
                return Err(MediaError::InvalidState(
                    "download speed must be greater than zero".to_string(),
                ));
            }
            Ok(MediaCommand::StartDownload {
                media_session_id,
                channel_id,
                device_id,
                target,
                call_id,
                local_tag,
                cseq,
                branch,
                subject_session,
                media_address,
                media_port,
                ssrc,
                transport,
                start_time: window.start_time,
                end_time: window.end_time,
                download_speed,
            })
        }
        GbMediaPurpose::Talk => {
            let codec = codec.ok_or_else(|| {
                MediaError::Unsupported("talk requires an audio codec".to_string())
            })?;
            if !SUPPORTED_TALK_CODECS.contains(&codec.as_str()) {
                return Err(MediaError::Unsupported(codec));
            }
            if ssrc.is_some() {
                return Err(MediaError::InvalidState(
                    "talk audio sessions must not carry a video SSRC".to_string(),
                ));
            }
            Ok(MediaCommand::StartTalk {
                media_session_id,
                channel_id,
                device_id,
                target,
                call_id,
                local_tag,
                cseq,
                branch,
                subject_session,
                media_address,
                media_port,
                codec,
                transport,
            })
        }
    }
}

/// Maps a control-plane [`MediaControl`] into a playback control command.
///
/// The MANSRTSP body carried by the resulting [`MediaCommand::ControlPlayback`]
/// is built from the action and, for seek/scale, a `Range`/`Scale` field:
///
/// - [`MediaControl::Play`] -> `PLAY`
/// - [`MediaControl::Pause`] -> `PAUSE`
/// - [`MediaControl::Stop`] -> `TEARDOWN`
/// - [`MediaControl::Seek`] -> `PLAY` with `Range: npt=<seconds>-`
/// - [`MediaControl::Scale`] -> `PLAY` with `Scale: <value>`
///
/// A non-finite or non-positive seek offset, and a non-finite scale, return
/// [`MediaError::InvalidState`]. Unknown future controls return
/// [`MediaError::Unsupported`].
pub fn map_control(
    media_session_id: MediaSessionId,
    control: &MediaControl,
) -> Result<MediaCommand, MediaError> {
    let (action, scale, range) = match control {
        MediaControl::Play => (PlaybackAction::Play, None, None),
        MediaControl::Pause => (PlaybackAction::Pause, None, None),
        MediaControl::Stop => (PlaybackAction::Teardown, None, None),
        MediaControl::Seek { offset_ms } => {
            if *offset_ms < 0 {
                return Err(MediaError::InvalidState(
                    "seek offset must not be negative".to_string(),
                ));
            }
            // MANSRTSP Range uses NPT seconds relative to the recording start.
            // Preserve the millisecond precision the domain models: whole
            // seconds render as a plain integer, sub-second offsets keep three
            // fractional digits (e.g. `npt=1.500-`).
            let seconds = *offset_ms / 1000;
            let millis = *offset_ms % 1000;
            let npt = if millis == 0 {
                format!("npt={seconds}-")
            } else {
                format!("npt={seconds}.{millis:03}-")
            };
            (PlaybackAction::Play, None, Some(npt))
        }
        MediaControl::Scale { value } => {
            if !value.is_finite() {
                return Err(MediaError::InvalidState(
                    "scale value must be finite".to_string(),
                ));
            }
            (PlaybackAction::Play, Some(*value), None)
        }
        other => {
            return Err(MediaError::Unsupported(format!(
                "playback control {other:?} is not supported"
            )));
        }
    };

    Ok(MediaCommand::ControlPlayback {
        media_session_id,
        action,
        scale,
        range,
    })
}

/// Requires a video SSRC for live/playback/download sessions.
fn require_video_ssrc(ssrc: Option<String>) -> Result<String, MediaError> {
    ssrc.ok_or_else(|| {
        MediaError::InvalidState("video media session requires a negotiated SSRC".to_string())
    })
}

/// Requires a recording window for playback/download sessions.
fn require_window(
    window: Option<GbRecordWindow>,
    purpose: GbMediaPurpose,
) -> Result<GbRecordWindow, MediaError> {
    window
        .ok_or_else(|| MediaError::InvalidState(format!("{purpose:?} requires a recording window")))
}

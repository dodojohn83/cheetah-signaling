//! GB28181 media session state machine (INVITE/ACK/BYE/SDP).
//!
//! This module maps a generic media command (start/stop live, playback, etc.)
//! into SIP request/response sequences. It does not perform network I/O; all
//! wire messages are returned inside [`MediaOutput::SendMessage`] for the
//! transport driver to send.

pub(crate) mod commands;
pub(crate) mod control;
pub(crate) mod handlers;
pub(crate) mod invite;
pub mod mapper;
pub(crate) mod session;

#[cfg(test)]
mod tests;

pub use control::PlaybackAction;
pub use mapper::{
    GbMediaEndpoint, GbMediaPurpose, GbRecordWindow, GbSipRouting, GbStartRequest, map_control,
    map_start,
};

use crate::events::Gb28181Event;
use crate::types::{DeviceId, DomainId};
use cheetah_gb28181_core::{CompatibilityProfile, SipMessage, SipUri};
use cheetah_signal_types::{ChannelId, MediaSessionId};
use std::collections::BTreeMap;

/// Transport negotiated for a media session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaTransport {
    /// UDP/RTP (GB28181 default for live).
    Udp,
    /// TCP/RTP where the media node accepts and the device initiates.
    TcpPassive,
    /// TCP/RTP where the media node initiates and the device accepts.
    TcpActive,
}

impl MediaTransport {
    /// Returns the `m=` line transport token.
    pub fn proto(self) -> &'static str {
        match self {
            Self::Udp => "RTP/AVP",
            Self::TcpPassive | Self::TcpActive => "TCP/RTP/AVP",
        }
    }

    /// Returns true when this transport uses a TCP `a=setup` attribute.
    pub fn is_tcp(self) -> bool {
        !matches!(self, Self::Udp)
    }
}

/// Configuration for the local GB28181 media UA.
#[derive(Clone, Debug)]
pub struct MediaConfig {
    /// Local SIP identity used in `Contact` and `From` headers.
    pub local_sip_uri: SipUri,
    /// Maximum number of concurrent media sessions tracked by this instance.
    pub max_sessions: usize,
    /// Local domain emitted in events.
    pub domain_id: DomainId,
    /// Compatibility profile gating controlled media-negotiation overrides.
    ///
    /// Defaults to the strict profile; SDP payload/attribute widening and
    /// broadcast address handling only apply when the matched profile enables
    /// the corresponding capability.
    pub compatibility: CompatibilityProfile,
}

/// Maximum byte length of the `c=`/origin media address in start commands.
pub(crate) const MAX_MEDIA_ADDRESS_BYTES: usize = 256;
/// Maximum byte length of the `a=y:` SSRC token.
pub(crate) const MAX_SSRC_BYTES: usize = 64;
/// Maximum byte length of SIP tokens reused in start commands (Call-ID, tags,
/// branch, Subject).
pub(crate) const MAX_SIP_TOKEN_BYTES: usize = 256;
/// Maximum byte length of a recording window epoch token.
pub(crate) const MAX_RECORD_TIME_BYTES: usize = 64;
/// Maximum byte length of an audio codec token.
pub(crate) const MAX_CODEC_BYTES: usize = 64;
/// Maximum byte length of a playback range token.
pub(crate) const MAX_RANGE_BYTES: usize = 256;

/// Command that drives a GB28181 media session.
#[allow(clippy::large_enum_variant, missing_docs)]
#[derive(Clone, Debug)]
pub enum MediaCommand {
    /// Start a live preview session.
    StartLive {
        media_session_id: MediaSessionId,
        channel_id: ChannelId,
        device_id: DeviceId,
        target: SipUri,
        call_id: String,
        local_tag: String,
        cseq: u32,
        branch: String,
        subject_session: String,
        media_address: String,
        media_port: u16,
        ssrc: String,
        transport: MediaTransport,
    },
    /// Start a recorded video playback session.
    StartPlayback {
        media_session_id: MediaSessionId,
        channel_id: ChannelId,
        device_id: DeviceId,
        target: SipUri,
        call_id: String,
        local_tag: String,
        cseq: u32,
        branch: String,
        subject_session: String,
        media_address: String,
        media_port: u16,
        ssrc: String,
        transport: MediaTransport,
        start_time: String,
        end_time: String,
    },
    /// Start a recorded video download session.
    StartDownload {
        media_session_id: MediaSessionId,
        channel_id: ChannelId,
        device_id: DeviceId,
        target: SipUri,
        call_id: String,
        local_tag: String,
        cseq: u32,
        branch: String,
        subject_session: String,
        media_address: String,
        media_port: u16,
        ssrc: String,
        transport: MediaTransport,
        start_time: String,
        end_time: String,
        download_speed: u32,
    },
    /// Start a two-way voice talk session.
    StartTalk {
        media_session_id: MediaSessionId,
        channel_id: ChannelId,
        device_id: DeviceId,
        target: SipUri,
        call_id: String,
        local_tag: String,
        cseq: u32,
        branch: String,
        subject_session: String,
        media_address: String,
        media_port: u16,
        codec: String,
        transport: MediaTransport,
    },
    /// Start a one-way voice broadcast session (platform sends audio to the
    /// device via a `sendonly` audio dialog).
    StartBroadcast {
        media_session_id: MediaSessionId,
        channel_id: ChannelId,
        device_id: DeviceId,
        target: SipUri,
        call_id: String,
        local_tag: String,
        cseq: u32,
        branch: String,
        subject_session: String,
        media_address: String,
        media_port: u16,
        codec: String,
        transport: MediaTransport,
    },
    /// Send a playback control command on an active playback dialog.
    ControlPlayback {
        media_session_id: MediaSessionId,
        action: PlaybackAction,
        scale: Option<f64>,
        range: Option<String>,
    },
    /// Stop or cancel an established or pending media session.
    StopMediaSession { media_session_id: MediaSessionId },
}

#[allow(clippy::too_many_arguments)]
fn validate_start_command(
    call_id: &str,
    local_tag: &str,
    branch: &str,
    subject_session: &str,
    media_address: &str,
    ssrc: Option<&str>,
    start_time: Option<&str>,
    end_time: Option<&str>,
    codec: Option<&str>,
) -> Result<(), MediaError> {
    let check = |s: &str, max: usize, msg: &'static str| -> Result<(), MediaError> {
        if s.len() > max {
            Err(MediaError::InvalidState(msg.to_string()))
        } else {
            Ok(())
        }
    };

    check(
        call_id,
        MAX_SIP_TOKEN_BYTES,
        "call_id exceeds maximum length",
    )?;
    check(
        local_tag,
        MAX_SIP_TOKEN_BYTES,
        "local_tag exceeds maximum length",
    )?;
    check(branch, MAX_SIP_TOKEN_BYTES, "branch exceeds maximum length")?;
    check(
        subject_session,
        MAX_SIP_TOKEN_BYTES,
        "subject_session exceeds maximum length",
    )?;
    check(
        media_address,
        MAX_MEDIA_ADDRESS_BYTES,
        "media_address exceeds maximum length",
    )?;
    if let Some(ssrc) = ssrc {
        check(ssrc, MAX_SSRC_BYTES, "ssrc exceeds maximum length")?;
    }
    if let Some(start_time) = start_time {
        check(
            start_time,
            MAX_RECORD_TIME_BYTES,
            "start_time exceeds maximum length",
        )?;
    }
    if let Some(end_time) = end_time {
        check(
            end_time,
            MAX_RECORD_TIME_BYTES,
            "end_time exceeds maximum length",
        )?;
    }
    if let Some(codec) = codec {
        check(codec, MAX_CODEC_BYTES, "codec exceeds maximum length")?;
    }
    Ok(())
}

impl MediaCommand {
    /// Validates that all string fields are bounded before the command is used
    /// to build SIP/SDP output or stored in the session table.
    pub(crate) fn validate(&self) -> Result<(), MediaError> {
        match self {
            Self::StartLive {
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                ssrc,
                ..
            } => validate_start_command(
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                Some(ssrc),
                None,
                None,
                None,
            ),
            Self::StartPlayback {
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                ssrc,
                start_time,
                end_time,
                ..
            } => validate_start_command(
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                Some(ssrc),
                Some(start_time),
                Some(end_time),
                None,
            ),
            Self::StartDownload {
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                ssrc,
                start_time,
                end_time,
                ..
            } => validate_start_command(
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                Some(ssrc),
                Some(start_time),
                Some(end_time),
                None,
            ),
            Self::StartTalk {
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                codec,
                ..
            } => validate_start_command(
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                None,
                None,
                None,
                Some(codec),
            ),
            Self::StartBroadcast {
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                codec,
                ..
            } => validate_start_command(
                call_id,
                local_tag,
                branch,
                subject_session,
                media_address,
                None,
                None,
                None,
                Some(codec),
            ),
            Self::ControlPlayback { range, .. } => {
                if let Some(range) = range
                    && range.len() > MAX_RANGE_BYTES
                {
                    return Err(MediaError::InvalidState(
                        "playback range exceeds maximum length".to_string(),
                    ));
                }
                Ok(())
            }
            Self::StopMediaSession { .. } => Ok(()),
        }
    }
}

/// An input delivered to the media state machine.
#[derive(Clone, Debug)]
pub enum MediaInput {
    /// A high-level command from the application layer.
    Command(MediaCommand),
    /// A SIP message received from the network.
    Message(SipMessage),
    /// A periodic tick for timeout processing.
    Tick,
}

/// An output produced by the media state machine.
#[derive(Clone, Debug)]
pub enum MediaOutput {
    /// A SIP message that the transport should send.
    SendMessage(SipMessage),
    /// An event for downstream consumers.
    EmitEvent(Gb28181Event),
}

/// Errors returned by the media state machine.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum MediaError {
    /// Session table is full.
    #[error("session table is full")]
    SessionTableFull,
    /// Session not found.
    #[error("session not found")]
    SessionNotFound,
    /// Session is in an incompatible state for the command.
    #[error("invalid session state: {0}")]
    InvalidState(String),
    /// The SIP message cannot be used (missing Call-ID, malformed, etc.).
    #[error("malformed SIP message: {0}")]
    MalformedSip(String),
    /// The SDP body is missing or malformed.
    #[error("malformed SDP: {0}")]
    MalformedSdp(String),
    /// Session already exists.
    #[error("session already exists")]
    AlreadyExists,
    /// Capability or codec is not supported.
    #[error("unsupported capability: {0}")]
    Unsupported(String),
}

/// Sans-I/O state machine for GB28181 media sessions.
#[derive(Clone, Debug)]
pub struct Gb28181Media {
    config: MediaConfig,
    sessions: BTreeMap<MediaSessionId, session::Session>,
    call_index: BTreeMap<String, MediaSessionId>,
}

impl Gb28181Media {
    /// Creates a new media state machine.
    pub fn new(config: MediaConfig) -> Self {
        Self {
            config,
            sessions: BTreeMap::new(),
            call_index: BTreeMap::new(),
        }
    }

    /// Removes a session and its Call-ID index entry, returning it.
    pub(crate) fn remove_session(&mut self, sid: MediaSessionId) -> Option<session::Session> {
        let session = self.sessions.remove(&sid)?;
        self.call_index.remove(&session.call_id);
        Some(session)
    }

    /// Processes an input and returns ordered outputs.
    pub fn process(&mut self, input: MediaInput) -> Result<Vec<MediaOutput>, MediaError> {
        match input {
            MediaInput::Command(cmd) => commands::on_command(self, cmd),
            MediaInput::Message(msg) => handlers::on_message(self, msg),
            MediaInput::Tick => Ok(Vec::new()),
        }
    }
}

//! GB28181 media session state machine (INVITE/ACK/BYE/SDP).
//!
//! This module maps a generic media command (start/stop live, playback, etc.)
//! into SIP request/response sequences. It does not perform network I/O; all
//! wire messages are returned inside [`MediaOutput::SendMessage`] for the
//! transport driver to send.

// Clippy suggests replacing `|e| MediaError::malformed_sip(e)` with the
// function item, but the constructors are generic over `impl Display` and
// cannot be used as ordinary function pointers in `map_err`.
#![allow(clippy::redundant_closure)]

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

/// Maximum byte length of the human-readable message carried by a `MediaError`.
const MAX_MEDIA_ERROR_BYTES: usize = 1024;

/// Truncates `message` at a UTF-8 character boundary so it is at most
/// `MAX_MEDIA_ERROR_BYTES` long.
fn clamp_message(message: impl std::fmt::Display) -> String {
    let s = message.to_string();
    if s.len() <= MAX_MEDIA_ERROR_BYTES {
        return s;
    }
    let mut idx = MAX_MEDIA_ERROR_BYTES;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    s[..idx].to_string()
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

impl MediaError {
    /// Creates an `InvalidState` error with a clamped message.
    pub fn invalid_state(message: impl std::fmt::Display) -> Self {
        Self::InvalidState(clamp_message(message))
    }

    /// Creates a `MalformedSip` error with a clamped message.
    pub fn malformed_sip(message: impl std::fmt::Display) -> Self {
        Self::MalformedSip(clamp_message(message))
    }

    /// Creates a `MalformedSdp` error with a clamped message.
    pub fn malformed_sdp(message: impl std::fmt::Display) -> Self {
        Self::MalformedSdp(clamp_message(message))
    }

    /// Creates an `Unsupported` error with a clamped message.
    pub fn unsupported(message: impl std::fmt::Display) -> Self {
        Self::Unsupported(clamp_message(message))
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn constructors_passthrough_short_messages() {
        assert_eq!(
            MediaError::invalid_state("ready").to_string(),
            "invalid session state: ready"
        );
        assert_eq!(
            MediaError::malformed_sip("missing tag").to_string(),
            "malformed SIP message: missing tag"
        );
        assert_eq!(
            MediaError::malformed_sdp("no c=").to_string(),
            "malformed SDP: no c="
        );
        assert_eq!(
            MediaError::unsupported("g711").to_string(),
            "unsupported capability: g711"
        );
    }

    #[test]
    fn constructors_clamp_long_messages_at_char_boundary() {
        let padding = "x".repeat(MAX_MEDIA_ERROR_BYTES);
        let trailer = "\u{1F600}";
        let message = format!("{padding}{trailer}");
        let err = MediaError::invalid_state(message.clone());
        let inner = match err {
            MediaError::InvalidState(s) => s,
            _ => panic!("expected InvalidState"),
        };
        assert!(inner.len() <= MAX_MEDIA_ERROR_BYTES);
        assert!(inner.is_char_boundary(inner.len()));
        assert!(message.starts_with(&inner));
    }
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

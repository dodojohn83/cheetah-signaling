//! GB28181 media session state machine (INVITE/ACK/BYE/SDP).
//!
//! This module maps a generic media command (start/stop live, playback, etc.)
//! into SIP request/response sequences. It does not perform network I/O; all
//! wire messages are returned inside [`MediaOutput::SendMessage`] for the
//! transport driver to send.

pub(crate) mod invite;
pub(crate) mod session;

#[cfg(test)]
mod tests;

use self::invite::{
    build_ack, build_bye, build_invite, build_ok_response, first_contact_uri, tag_from_header,
};
use self::session::{Session, SessionState, failed_event, socket_addr, stopped_event};
use crate::events::Gb28181Event;
use crate::types::{DeviceId, DomainId};
use cheetah_gb28181_core::{HeaderName, Method, SipMessage, SipUri};
use cheetah_signal_types::{ChannelId, MediaSessionId};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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
}

/// Command that drives a GB28181 media session.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum MediaCommand {
    /// Start a live preview session.
    StartLive {
        /// Media session identifier.
        media_session_id: MediaSessionId,
        /// Channel identifier.
        channel_id: ChannelId,
        /// GB28181 device identifier (numeric, from the SIP URI user part).
        device_id: DeviceId,
        /// Target URI for the INVITE (device Contact/Record-Route resolved).
        target: SipUri,
        /// `Call-ID` for the INVITE dialog.
        call_id: String,
        /// Local tag for the `From` header.
        local_tag: String,
        /// Initial `CSeq` number.
        cseq: u32,
        /// Top `Via` branch parameter.
        branch: String,
        /// GB28181 subject session identifier (numeric, placed in `Subject`).
        subject_session: String,
        /// Media node address advertised in SDP `c=`.
        media_address: String,
        /// Media node port advertised in SDP `m=`.
        media_port: u16,
        /// SSRC string for the `a=y:` attribute (20-digit GB28181 SSRC).
        ssrc: String,
        /// Transport protocol advertised in the SDP.
        transport: MediaTransport,
    },
    /// Stop an established or pending media session.
    StopLive {
        /// Media session identifier.
        media_session_id: MediaSessionId,
    },
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
}

/// Sans-I/O state machine for GB28181 media sessions.
#[derive(Clone, Debug)]
pub struct Gb28181Media {
    config: MediaConfig,
    sessions: BTreeMap<MediaSessionId, Session>,
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
    fn remove_session(&mut self, sid: MediaSessionId) -> Option<Session> {
        let session = self.sessions.remove(&sid)?;
        self.call_index.remove(&session.call_id);
        Some(session)
    }

    /// Processes an input and returns ordered outputs.
    pub fn process(&mut self, input: MediaInput) -> Result<Vec<MediaOutput>, MediaError> {
        match input {
            MediaInput::Command(cmd) => self.on_command(cmd),
            MediaInput::Message(msg) => self.on_message(msg),
            MediaInput::Tick => Ok(Vec::new()),
        }
    }

    fn on_command(&mut self, cmd: MediaCommand) -> Result<Vec<MediaOutput>, MediaError> {
        match cmd {
            MediaCommand::StartLive {
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
            } => {
                if self.sessions.len() >= self.config.max_sessions {
                    return Err(MediaError::SessionTableFull);
                }
                if self.sessions.contains_key(&media_session_id) {
                    return Err(MediaError::AlreadyExists);
                }

                let invite = build_invite(
                    &self.config.local_sip_uri,
                    &target,
                    &call_id,
                    &local_tag,
                    cseq,
                    &branch,
                    &device_id,
                    &subject_session,
                    &media_address,
                    media_port,
                    &ssrc,
                    transport,
                )
                .map_err(|e| MediaError::MalformedSip(e.to_string()))?;

                self.sessions.insert(
                    media_session_id,
                    Session {
                        media_session_id,
                        channel_id,
                        device_id,
                        call_id: call_id.clone(),
                        local_tag,
                        remote_tag: None,
                        cseq,
                        branch,
                        target,
                        remote_target: None,
                        state: SessionState::Inviting,
                        media_address,
                        media_port,
                    },
                );
                self.call_index.insert(call_id, media_session_id);

                Ok(vec![MediaOutput::SendMessage(invite)])
            }
            MediaCommand::StopLive { media_session_id } => {
                let session = self
                    .sessions
                    .get_mut(&media_session_id)
                    .ok_or(MediaError::SessionNotFound)?;
                if session.state == SessionState::Stopping
                    || session.state == SessionState::Terminated
                {
                    return Err(MediaError::InvalidState(format!("{:?}", session.state)));
                }

                session.cseq += 1;
                let branch = format!("{}-bye", session.branch);
                let target = session.remote_target.as_ref().unwrap_or(&session.target);
                let bye = build_bye(
                    &self.config.local_sip_uri,
                    session,
                    session.cseq,
                    &branch,
                    target,
                )
                .map_err(|e| MediaError::MalformedSip(e.to_string()))?;
                session.state = SessionState::Stopping;

                Ok(vec![MediaOutput::SendMessage(bye)])
            }
        }
    }

    fn on_message(&mut self, msg: SipMessage) -> Result<Vec<MediaOutput>, MediaError> {
        let call_id = msg
            .call_id()
            .ok_or_else(|| MediaError::MalformedSip("missing Call-ID".to_string()))?
            .to_string();
        let sid = *self
            .call_index
            .get(&call_id)
            .ok_or(MediaError::SessionNotFound)?;

        match &msg {
            SipMessage::Response { line, .. } => self.on_response(sid, line.code, msg.clone()),
            SipMessage::Request { line, .. } => self.on_request(sid, &line.method, msg.clone()),
        }
    }

    fn on_response(
        &mut self,
        sid: MediaSessionId,
        code: u16,
        msg: SipMessage,
    ) -> Result<Vec<MediaOutput>, MediaError> {
        let cseq = msg
            .cseq()
            .ok_or_else(|| MediaError::MalformedSip("missing or malformed CSeq".to_string()))?;

        if cseq.1 == Method::Invite {
            let cseq_match = self
                .sessions
                .get(&sid)
                .map(|s| s.cseq == cseq.0)
                .unwrap_or(false);
            if cseq_match {
                if (200..300).contains(&code) {
                    return self.on_invite_success(sid, msg);
                }
                if code >= 300 {
                    let session = self
                        .remove_session(sid)
                        .ok_or(MediaError::SessionNotFound)?;
                    let event = failed_event(&session, &self.config.domain_id, "invite rejected");
                    return Ok(vec![MediaOutput::EmitEvent(event)]);
                }
                // 1xx provisional: no action yet.
                return Ok(Vec::new());
            }
        }

        if cseq.1 == Method::Bye {
            let session = self
                .remove_session(sid)
                .ok_or(MediaError::SessionNotFound)?;
            let event = stopped_event(&session, &self.config.domain_id);
            return Ok(vec![MediaOutput::EmitEvent(event)]);
        }

        Ok(Vec::new())
    }

    fn on_request(
        &mut self,
        sid: MediaSessionId,
        method: &Method,
        msg: SipMessage,
    ) -> Result<Vec<MediaOutput>, MediaError> {
        if method == &Method::Bye {
            let session = self
                .remove_session(sid)
                .ok_or(MediaError::SessionNotFound)?;
            let ok = build_ok_response(&msg);
            let event = stopped_event(&session, &self.config.domain_id);
            return Ok(vec![
                MediaOutput::SendMessage(ok),
                MediaOutput::EmitEvent(event),
            ]);
        }
        // CANCEL for an outstanding INVITE is handled by the transaction layer.
        Ok(Vec::new())
    }

    fn on_invite_success(
        &mut self,
        sid: MediaSessionId,
        msg: SipMessage,
    ) -> Result<Vec<MediaOutput>, MediaError> {
        let session = self
            .sessions
            .get_mut(&sid)
            .ok_or(MediaError::SessionNotFound)?;

        let remote_tag = tag_from_header(&msg, &HeaderName::To)
            .ok_or_else(|| MediaError::MalformedSip("missing To tag in 200 OK".to_string()))?;
        let contact = first_contact_uri(&msg)?;
        let parsed_remote_sdp = cheetah_gb28181_core::parse_sdp(msg.body(), &Default::default())
            .map_err(|e| MediaError::MalformedSdp(e.to_string()))?;
        let remote_sdp_text = String::from_utf8_lossy(msg.body()).to_string();

        let remote_ssrc = parsed_remote_sdp
            .media
            .first()
            .and_then(|m| m.y_ssrc().map(|s| s.to_string()));
        let remote_proto = parsed_remote_sdp
            .media
            .first()
            .map(|m| m.proto.clone())
            .unwrap_or_default();
        let remote_port = parsed_remote_sdp.media.first().map(|m| m.port).unwrap_or(0);
        let remote_address = parsed_remote_sdp
            .connection
            .as_ref()
            .or_else(|| {
                parsed_remote_sdp
                    .media
                    .first()
                    .and_then(|m| m.connection.as_ref())
            })
            .map(|c| c.address.clone())
            .unwrap_or_default();

        session.remote_tag = Some(remote_tag.clone());
        session.remote_target = Some(contact.clone());

        let ack_branch = format!("{}-ack", session.branch);
        let ack = build_ack(
            &self.config.local_sip_uri,
            session,
            &remote_tag,
            &contact,
            &ack_branch,
        );

        session.state = SessionState::Active;

        let source = socket_addr(&remote_address, remote_port)
            .unwrap_or(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));

        let event = Gb28181Event::MediaSessionStarted {
            domain_id: self.config.domain_id.clone(),
            media_session_id: session.media_session_id,
            channel_id: session.channel_id,
            device_id: session.device_id.clone(),
            source,
            remote_sdp: remote_sdp_text,
            remote_ssrc,
            remote_port,
            remote_proto,
        };

        Ok(vec![
            MediaOutput::SendMessage(ack),
            MediaOutput::EmitEvent(event),
        ])
    }
}

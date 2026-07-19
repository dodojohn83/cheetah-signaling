//! Internal per-session protocol state for GB28181 media handling.

use crate::events::Gb28181Event;
use crate::types::{DeviceId, DomainId};
use cheetah_gb28181_core::SipUri;
use cheetah_signal_types::{ChannelId, MediaSessionId};
use std::net::{IpAddr, SocketAddr};

/// Lifecycle state of a tracked media session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    /// INVITE sent, waiting for a final response.
    Inviting,
    /// 200 OK received and ACK sent.
    Active,
    /// BYE sent, waiting for a final response, or device sent BYE.
    Stopping,
    /// Session has been removed from the table.
    Terminated,
}

/// Per-session protocol state.
#[derive(Clone, Debug)]
pub struct Session {
    pub media_session_id: MediaSessionId,
    pub channel_id: ChannelId,
    pub device_id: DeviceId,
    pub call_id: String,
    pub local_tag: String,
    pub remote_tag: Option<String>,
    /// CSeq counter used for in-dialog requests; incremented after each request.
    pub cseq: u32,
    /// CSeq number of the original INVITE transaction, used to match 2xx retransmissions.
    pub invite_cseq: u32,
    pub branch: String,
    /// Original remote AOR used in the To header.
    pub target: SipUri,
    /// Remote Contact URI used for in-dialog Request-URIs (ACK/BYE).
    pub remote_target: Option<SipUri>,
    pub state: SessionState,
    pub media_address: String,
    pub media_port: u16,
}

/// Parses a numeric IP address and port into a [`SocketAddr`].
pub fn socket_addr(host: &str, port: u16) -> Result<SocketAddr, super::MediaError> {
    let ip = host
        .parse::<IpAddr>()
        .map_err(|e| super::MediaError::MalformedSip(e.to_string()))?;
    Ok(SocketAddr::new(ip, port))
}

/// Builds a `MediaSessionStopped` event from session state.
pub fn stopped_event(session: &Session, domain_id: &DomainId) -> Gb28181Event {
    let source = socket_addr(&session.media_address, session.media_port).ok();

    Gb28181Event::MediaSessionStopped {
        domain_id: domain_id.clone(),
        media_session_id: session.media_session_id,
        channel_id: session.channel_id,
        device_id: session.device_id.clone(),
        source,
    }
}

/// Builds a `MediaSessionFailed` event from session state.
pub fn failed_event(session: &Session, domain_id: &DomainId, reason: &str) -> Gb28181Event {
    let source = socket_addr(&session.media_address, session.media_port).ok();

    Gb28181Event::MediaSessionFailed {
        domain_id: domain_id.clone(),
        media_session_id: session.media_session_id,
        channel_id: session.channel_id,
        device_id: session.device_id.clone(),
        source,
        reason: reason.to_string(),
    }
}

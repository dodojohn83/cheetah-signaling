//! Domain events emitted by the GB28181 access module.

use crate::types::{DeviceId, DomainId};
use crate::xml::{CatalogItem, RecordItem};
use cheetah_signal_types::{ChannelId, MediaSessionId};
use std::net::SocketAddr;

/// Presence state reported by a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevicePresence {
    /// Device has registered or refreshed registration.
    Online,
    /// Device has explicitly unregistered or expired.
    Offline,
}

/// Events produced by the GB28181 module for downstream consumers.
#[derive(Clone, Debug)]
pub enum Gb28181Event {
    /// A device registered or refreshed registration.
    DeviceRegistered {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier from the SIP URI user part.
        device_id: DeviceId,
        /// Source address observed from the transport.
        source: SocketAddr,
        /// Parsed Contact endpoint (host:port) for subsequent requests.
        contact: String,
        /// Granted expiry in seconds.
        expires: u32,
        /// Raw User-Agent header, if present.
        user_agent: Option<String>,
    },
    /// A device explicitly unregistered or its registration expired.
    DeviceUnregistered {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier from the SIP URI user part.
        device_id: DeviceId,
        /// Source address observed from the transport.
        source: SocketAddr,
    },
    /// Device presence changed due to keepalive timeout or recovery.
    DevicePresenceChanged {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// New presence state.
        presence: DevicePresence,
    },
    /// A keepalive was received.
    Keepalive {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Parsed keepalive status.
        status: String,
    },
    /// A catalog response fragment was received.
    CatalogReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Declared total number of items across all fragments.
        sum_num: u32,
        /// Number of items in this fragment.
        num: u32,
        /// Items in this fragment.
        items: Vec<CatalogItem>,
    },
    /// A device info response was received.
    DeviceInfoReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Result string, if present.
        result: Option<String>,
        /// Manufacturer, if present.
        manufacturer: Option<String>,
        /// Model, if present.
        model: Option<String>,
        /// Firmware version, if present.
        firmware: Option<String>,
    },
    /// A device status response was received.
    DeviceStatusReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Result string, if present.
        result: Option<String>,
        /// Online state, if present.
        online: Option<String>,
        /// Status, if present.
        status: Option<String>,
        /// Reason, if present.
        reason: Option<String>,
        /// Invalid equipment flag, if present.
        invalid_equip: Option<String>,
    },
    /// An alarm notification was received.
    AlarmReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Alarm priority.
        priority: Option<String>,
        /// Alarm method.
        method: Option<String>,
        /// Alarm type.
        alarm_type: Option<String>,
        /// Alarm time.
        time: Option<String>,
        /// Extended alarm information.
        info: Option<String>,
    },
    /// A mobile position report was received.
    MobilePositionReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Report time.
        time: Option<String>,
        /// Longitude.
        longitude: Option<String>,
        /// Latitude.
        latitude: Option<String>,
        /// Speed.
        speed: Option<String>,
        /// Direction.
        direction: Option<String>,
        /// Altitude.
        altitude: Option<String>,
    },
    /// A record info response fragment was received.
    RecordInfoReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Device name, if present.
        name: Option<String>,
        /// Declared total number of records across all fragments.
        sum_num: u32,
        /// Number of records in this fragment.
        num: u32,
        /// Records in this fragment.
        items: Vec<RecordItem>,
    },
    /// A `DeviceControl` response was received (ACK/result for a PTZ or
    /// preset command sent earlier).
    DeviceControlResponseReceived {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Device identifier.
        device_id: DeviceId,
        /// Source address.
        source: SocketAddr,
        /// Sequence number.
        sn: String,
        /// Result reported by the device, if any.
        result: Option<String>,
    },
    /// A live or playback media session was successfully established.
    MediaSessionStarted {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Media session identifier from the originating command.
        media_session_id: MediaSessionId,
        /// Channel identifier.
        channel_id: ChannelId,
        /// Device identifier.
        device_id: DeviceId,
        /// Remote media address (from the device SDP connection line).
        source: SocketAddr,
        /// Raw remote SDP answer.
        remote_sdp: String,
        /// SSRC reported by the device, if present.
        remote_ssrc: Option<String>,
        /// Remote media port.
        remote_port: u16,
        /// Negotiated transport protocol (e.g. `RTP/AVP` or `TCP/RTP/AVP`).
        remote_proto: String,
    },
    /// A media session was torn down.
    MediaSessionStopped {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Media session identifier.
        media_session_id: MediaSessionId,
        /// Channel identifier.
        channel_id: ChannelId,
        /// Device identifier.
        device_id: DeviceId,
        /// Remote address that was used for the session.
        source: SocketAddr,
    },
    /// An upstream GB28181 cascade platform registered or refreshed.
    CascadePlatformConnected {
        /// Logical domain.
        domain_id: DomainId,
        /// Upstream platform identifier.
        platform_id: String,
        /// Upstream SIP URI that accepted the registration.
        upstream: String,
        /// Granted expiry in seconds.
        expires: u32,
    },
    /// An upstream GB28181 cascade platform explicitly unregistered or failed.
    CascadePlatformDisconnected {
        /// Logical domain.
        domain_id: DomainId,
        /// Upstream platform identifier.
        platform_id: String,
        /// Stable reason for the disconnection.
        reason: String,
    },
    /// A media session establishment or operation failed.
    MediaSessionFailed {
        /// Logical domain the device belongs to.
        domain_id: DomainId,
        /// Media session identifier.
        media_session_id: MediaSessionId,
        /// Channel identifier.
        channel_id: ChannelId,
        /// Device identifier.
        device_id: DeviceId,
        /// Remote address, if known.
        source: Option<SocketAddr>,
        /// Stable failure reason.
        reason: String,
    },
    /// An upstream cascade platform sent an INVITE to play a channel.
    CascadePlayRequested {
        /// Logical domain.
        domain_id: DomainId,
        /// Upstream platform identifier.
        platform_id: String,
        /// Stable bridge identifier chosen by the cascade state machine.
        bridge_id: String,
        /// Upstream Call-ID for the INVITE transaction.
        upstream_call_id: String,
        /// Upstream From URI with tag, as a string.
        upstream_from: String,
        /// Upstream To URI.
        upstream_to: String,
        /// Target user part from the upstream Request-URI (external device/channel ID).
        target_user: String,
        /// Parsed remote SDP offer.
        remote_sdp: String,
    },
    /// An upstream cascade play bridge was torn down by BYE/CANCEL or the
    /// downstream side stopped.
    CascadePlayStopped {
        /// Logical domain.
        domain_id: DomainId,
        /// Upstream platform identifier.
        platform_id: String,
        /// Stable bridge identifier.
        bridge_id: String,
        /// Stable reason for the stop.
        reason: String,
    },
}

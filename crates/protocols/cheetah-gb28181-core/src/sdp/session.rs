//! SDP session and media descriptors for GB28181.

/// An SDP session description.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SdpSession {
    /// Protocol version, normally `0`.
    pub version: String,
    /// Session origin.
    pub origin: SdpOrigin,
    /// Session name.
    pub name: String,
    /// Optional session information.
    pub info: Option<String>,
    /// Session-level connection, if present.
    pub connection: Option<SdpConnection>,
    /// Time descriptions.
    pub times: Vec<SdpTime>,
    /// Media descriptions.
    pub media: Vec<SdpMedia>,
    /// Session-level attributes.
    pub attributes: Vec<SdpAttribute>,
}

/// SDP origin (`o=` line).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SdpOrigin {
    /// Username.
    pub username: String,
    /// Session identifier.
    pub sess_id: String,
    /// Session version.
    pub sess_version: String,
    /// Network type, normally `IN`.
    pub nettype: String,
    /// Address type, e.g. `IP4` or `IP6`.
    pub addrtype: String,
    /// Unicast address.
    pub address: String,
}

/// SDP connection (`c=` line).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SdpConnection {
    /// Network type.
    pub nettype: String,
    /// Address type.
    pub addrtype: String,
    /// Address; may be multicast with TTL or count suffix.
    pub address: String,
}

/// SDP time description (`t=` line).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SdpTime {
    /// Start time (0 means session is permanent/permanent until stopped).
    pub start: String,
    /// Stop time.
    pub stop: String,
}

/// A single media description (`m=` line and its attributes).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SdpMedia {
    /// Media type, e.g. `video`, `audio`, `application`.
    pub media_type: String,
    /// Transport port.
    pub port: u16,
    /// Number of ports (rarely used; default 1).
    pub port_count: u16,
    /// Transport protocol, e.g. `RTP/AVP`, `TCP/RTP/AVP`.
    pub proto: String,
    /// Payload type list.
    pub formats: Vec<String>,
    /// Media-level connection, if present.
    pub connection: Option<SdpConnection>,
    /// Media attributes.
    pub attributes: Vec<SdpAttribute>,
    /// Media title, if present.
    pub title: Option<String>,
}

impl Default for SdpMedia {
    fn default() -> Self {
        Self {
            media_type: String::new(),
            port: 0,
            port_count: 1,
            proto: String::new(),
            formats: Vec::new(),
            connection: None,
            attributes: Vec::new(),
            title: None,
        }
    }
}

/// RTP mapping for a payload type (`a=rtpmap:`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RtpMap {
    /// Payload type number.
    pub pt: String,
    /// Encoding name.
    pub encoding: String,
    /// Clock rate in Hz.
    pub clock: String,
    /// Optional encoding parameters, e.g. number of channels.
    pub params: Option<String>,
}

/// `a=setup` attribute value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdpSetup {
    /// Active (initiate connection).
    Active,
    /// Passive (accept connection).
    Passive,
    /// Both active and passive acceptable.
    Actpass,
    /// None.
    None,
}

/// `a=connection` attribute value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdpConnectionType {
    /// New connection.
    New,
    /// Existing connection.
    Existing,
}

/// A parsed SDP attribute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SdpAttribute {
    /// `a=rtpmap:<pt> <encoding>/<clock>[/<params>]`
    RtpMap(RtpMap),
    /// `a=fmtp:<pt> <params>`
    Fmtp {
        /// Payload type.
        pt: String,
        /// Format parameters.
        params: String,
    },
    /// `a=setup:<active|passive|actpass|none>`
    Setup(SdpSetup),
    /// `a=connection:<new|existing>`
    Connection(SdpConnectionType),
    /// `a=ssrc:<id> ...`
    Ssrc {
        /// SSRC identifier as a decimal string.
        id: String,
        /// Optional attribute text.
        text: Option<String>,
    },
    /// GB28181 SSRC string (`a=y:<ssrc>`).
    Y(String),
    /// Direction attribute (`sendonly`, `recvonly`, `sendrecv`, `inactive`).
    Direction(SdpDirection),
    /// Unknown attribute preserved as `name: value`.
    Unknown {
        /// Attribute name.
        name: String,
        /// Attribute value; `None` for value-less flag attributes.
        value: Option<String>,
    },
}

/// Media direction attribute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdpDirection {
    /// `sendonly`
    SendOnly,
    /// `recvonly`
    RecvOnly,
    /// `sendrecv`
    SendRecv,
    /// `inactive`
    Inactive,
}

impl SdpMedia {
    /// Returns the first `rtpmap` for the given payload type, if any.
    pub fn rtpmap_for(&self, pt: &str) -> Option<&RtpMap> {
        self.attributes.iter().find_map(|a| match a {
            SdpAttribute::RtpMap(r) if r.pt == pt => Some(r),
            _ => None,
        })
    }

    /// Returns all `fmtp` values for the given payload type.
    pub fn fmtp_for(&self, pt: &str) -> Vec<&str> {
        self.attributes
            .iter()
            .filter_map(|a| match a {
                SdpAttribute::Fmtp { pt: p, params } if p == pt => Some(params.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Returns the `setup` attribute if present.
    pub fn setup(&self) -> Option<SdpSetup> {
        self.attributes.iter().find_map(|a| match a {
            SdpAttribute::Setup(s) => Some(*s),
            _ => None,
        })
    }

    /// Returns the `connection` attribute if present.
    pub fn connection_attr(&self) -> Option<SdpConnectionType> {
        self.attributes.iter().find_map(|a| match a {
            SdpAttribute::Connection(c) => Some(*c),
            _ => None,
        })
    }

    /// Returns all `ssrc` attribute values.
    pub fn ssrcs(&self) -> Vec<&str> {
        self.attributes
            .iter()
            .filter_map(|a| match a {
                SdpAttribute::Ssrc { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Returns the GB28181 `y=` SSRC string if present.
    pub fn y_ssrc(&self) -> Option<&str> {
        self.attributes.iter().find_map(|a| match a {
            SdpAttribute::Y(v) => Some(v.as_str()),
            _ => None,
        })
    }
}

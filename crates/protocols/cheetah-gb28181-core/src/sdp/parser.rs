//! Streaming SDP parser with configurable limits.

use super::error::SdpError;
use super::session::{
    RtpMap, SdpAttribute, SdpConnection, SdpConnectionType, SdpDirection, SdpMedia, SdpOrigin,
    SdpSession, SdpSetup, SdpTime,
};

/// Parser limits for SDP bodies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SdpParserConfig {
    /// Maximum number of lines.
    pub max_lines: usize,
    /// Maximum length of one line.
    pub max_line_len: usize,
    /// Maximum total body size in bytes.
    pub max_size: usize,
    /// Maximum media descriptions.
    pub max_media: usize,
    /// Maximum attributes per media or session block.
    pub max_attributes: usize,
    /// Maximum unknown attributes to preserve per block.
    pub max_unknown_attributes: usize,
}

impl Default for SdpParserConfig {
    fn default() -> Self {
        Self {
            max_lines: 4096,
            max_line_len: 4096,
            max_size: 512 * 1024,
            max_media: 64,
            max_attributes: 256,
            max_unknown_attributes: 64,
        }
    }
}

impl SdpParserConfig {
    /// Returns a small limit configuration useful for tests.
    pub fn test() -> Self {
        Self {
            max_lines: 64,
            max_line_len: 256,
            max_size: 4096,
            max_media: 4,
            max_attributes: 8,
            max_unknown_attributes: 4,
        }
    }
}

/// Parses an SDP body into a structured session.
pub fn parse_sdp(body: &[u8], config: &SdpParserConfig) -> Result<SdpSession, SdpError> {
    if body.len() > config.max_size {
        return Err(SdpError::LimitExceeded(format!(
            "SDP body size {} exceeds max {}",
            body.len(),
            config.max_size
        )));
    }

    let text = std::str::from_utf8(body)
        .map_err(|_| SdpError::Malformed("SDP body is not valid UTF-8".to_string()))?;

    let raw = text.replace("\r\n", "\n");
    let lines: Vec<&str> = raw.lines().collect();
    if lines.len() > config.max_lines {
        return Err(SdpError::LimitExceeded(format!(
            "SDP line count {} exceeds max {}",
            lines.len(),
            config.max_lines
        )));
    }

    for line in &lines {
        if line.len() > config.max_line_len {
            return Err(SdpError::LimitExceeded(format!(
                "SDP line length {} exceeds max {}",
                line.len(),
                config.max_line_len
            )));
        }
    }

    ParserState::new(lines, config).parse()
}

struct ParserState<'a> {
    lines: Vec<&'a str>,
    pos: usize,
    config: &'a SdpParserConfig,
}

impl<'a> ParserState<'a> {
    fn new(lines: Vec<&'a str>, config: &'a SdpParserConfig) -> Self {
        Self {
            lines,
            pos: 0,
            config,
        }
    }

    fn parse(&mut self) -> Result<SdpSession, SdpError> {
        let mut session = SdpSession::default();
        let mut current_media: Option<SdpMedia> = None;
        let mut session_attr_count = 0;
        let mut session_unknown_count = 0;

        // Parse session-level required lines first.
        self.expect_version(&mut session)?;
        self.expect_origin(&mut session)?;
        self.expect_name(&mut session)?;

        while self.pos < self.lines.len() {
            let line = self.lines[self.pos];
            if line.is_empty() {
                self.pos += 1;
                continue;
            }
            let (kind, value) = split_line(line)?;

            match kind {
                'm' => {
                    if session.media.len() >= self.config.max_media {
                        return Err(SdpError::LimitExceeded(format!(
                            "media count exceeds {}",
                            self.config.max_media
                        )));
                    }
                    if let Some(media) = current_media.take() {
                        session.media.push(media);
                    }
                    current_media = Some(parse_media(value)?);
                }
                _ => {
                    if let Some(ref mut media) = current_media {
                        self.parse_media_line(kind, value, media)?;
                    } else {
                        self.parse_session_line(
                            kind,
                            value,
                            &mut session,
                            &mut session_attr_count,
                            &mut session_unknown_count,
                        )?;
                    }
                }
            }
            self.pos += 1;
        }

        if let Some(media) = current_media.take() {
            session.media.push(media);
        }

        Ok(session)
    }

    fn expect_version(&mut self, session: &mut SdpSession) -> Result<(), SdpError> {
        let line = self.peek_line("expected v= line at start")?;
        let (kind, value) = split_line(line)?;
        if kind != 'v' {
            return Err(SdpError::Malformed(format!(
                "expected v= line, got {kind}="
            )));
        }
        session.version = value.to_string();
        self.pos += 1;
        Ok(())
    }

    fn expect_origin(&mut self, session: &mut SdpSession) -> Result<(), SdpError> {
        let line = self.peek_line("expected o= line")?;
        let (kind, value) = split_line(line)?;
        if kind != 'o' {
            return Err(SdpError::Malformed(format!(
                "expected o= line, got {kind}="
            )));
        }
        session.origin = parse_origin(value)?;
        self.pos += 1;
        Ok(())
    }

    fn expect_name(&mut self, session: &mut SdpSession) -> Result<(), SdpError> {
        let line = self.peek_line("expected s= line")?;
        let (kind, value) = split_line(line)?;
        if kind != 's' {
            return Err(SdpError::Malformed(format!(
                "expected s= line, got {kind}="
            )));
        }
        session.name = value.to_string();
        self.pos += 1;
        Ok(())
    }

    fn peek_line(&self, message: &str) -> Result<&'a str, SdpError> {
        self.lines
            .get(self.pos)
            .copied()
            .ok_or_else(|| SdpError::Malformed(message.to_string()))
    }

    fn parse_session_line(
        &mut self,
        kind: char,
        value: &str,
        session: &mut SdpSession,
        attr_count: &mut usize,
        unknown_count: &mut usize,
    ) -> Result<(), SdpError> {
        match kind {
            'i' => session.info = Some(value.to_string()),
            'u' | 'e' | 'p' | 'b' | 'z' | 'k' | 'r' => {
                // Unsupported but standard SDP lines; ignore for the GB28181 subset.
            }
            'c' => session.connection = Some(parse_connection(value)?),
            't' => session.times.push(parse_time(value)?),
            'a' => {
                let attr = parse_attribute(value)?;
                if matches!(attr, SdpAttribute::Unknown { .. }) {
                    if *unknown_count >= self.config.max_unknown_attributes {
                        return Err(SdpError::LimitExceeded(
                            "too many unknown session attributes".to_string(),
                        ));
                    }
                    *unknown_count += 1;
                }
                if *attr_count >= self.config.max_attributes {
                    return Err(SdpError::LimitExceeded(
                        "too many session attributes".to_string(),
                    ));
                }
                *attr_count += 1;
                session.attributes.push(attr);
            }
            other => {
                return Err(SdpError::Malformed(format!(
                    "unexpected session-level line: {other}="
                )));
            }
        }
        Ok(())
    }

    fn parse_media_line(
        &mut self,
        kind: char,
        value: &str,
        media: &mut SdpMedia,
    ) -> Result<(), SdpError> {
        match kind {
            'i' => media.title = Some(value.to_string()),
            'c' => media.connection = Some(parse_connection(value)?),
            'b' | 'k' => {
                // Unsupported but standard lines; ignore.
            }
            'a' => {
                let attr = parse_attribute(value)?;
                if matches!(attr, SdpAttribute::Unknown { .. })
                    && media
                        .attributes
                        .iter()
                        .filter(|a| matches!(a, SdpAttribute::Unknown { .. }))
                        .count()
                        >= self.config.max_unknown_attributes
                {
                    return Err(SdpError::LimitExceeded(
                        "too many unknown media attributes".to_string(),
                    ));
                }
                if media.attributes.len() >= self.config.max_attributes {
                    return Err(SdpError::LimitExceeded(
                        "too many media attributes".to_string(),
                    ));
                }
                media.attributes.push(attr);
            }
            other => {
                return Err(SdpError::Malformed(format!(
                    "unexpected media-level line: {other}="
                )));
            }
        }
        Ok(())
    }
}

fn split_line(line: &str) -> Result<(char, &str), SdpError> {
    if line.is_empty() {
        return Err(SdpError::Malformed("empty SDP line".to_string()));
    }
    let mut chars = line.chars();
    let kind = chars
        .next()
        .ok_or_else(|| SdpError::Malformed("empty SDP line".to_string()))?;
    if chars.next() != Some('=') {
        return Err(SdpError::Malformed(format!("SDP line missing '=': {line}")));
    }
    let value = &line[kind.len_utf8() + '='.len_utf8()..];
    Ok((kind, value))
}

fn parse_origin(value: &str) -> Result<SdpOrigin, SdpError> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 6 {
        return Err(SdpError::Malformed(format!("invalid o= line: {value}")));
    }
    Ok(SdpOrigin {
        username: parts[0].to_string(),
        sess_id: parts[1].to_string(),
        sess_version: parts[2].to_string(),
        nettype: parts[3].to_string(),
        addrtype: parts[4].to_string(),
        address: parts[5].to_string(),
    })
}

fn parse_connection(value: &str) -> Result<SdpConnection, SdpError> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(SdpError::Malformed(format!("invalid c= line: {value}")));
    }
    Ok(SdpConnection {
        nettype: parts[0].to_string(),
        addrtype: parts[1].to_string(),
        address: parts[2].to_string(),
    })
}

fn parse_time(value: &str) -> Result<SdpTime, SdpError> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(SdpError::Malformed(format!("invalid t= line: {value}")));
    }
    Ok(SdpTime {
        start: parts[0].to_string(),
        stop: parts[1].to_string(),
    })
}

fn parse_media(value: &str) -> Result<SdpMedia, SdpError> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 4 {
        return Err(SdpError::Malformed(format!("invalid m= line: {value}")));
    }

    let media_type = parts[0].to_string();
    let (port, port_count) = parse_port(parts[1])?;
    let proto = parts[2].to_string();
    let formats = parts[3..].iter().map(|s| (*s).to_string()).collect();

    Ok(SdpMedia {
        media_type,
        port,
        port_count,
        proto,
        formats,
        connection: None,
        attributes: Vec::new(),
        title: None,
    })
}

fn parse_port(s: &str) -> Result<(u16, u16), SdpError> {
    if let Some(idx) = s.find('/') {
        let (port, count) = s.split_at(idx);
        let count = &count[1..];
        let port = port
            .parse::<u16>()
            .map_err(|_| SdpError::Malformed(format!("invalid port: {s}")))?;
        let count = count
            .parse::<u16>()
            .map_err(|_| SdpError::Malformed(format!("invalid port count: {s}")))?;
        Ok((port, count))
    } else {
        let port = s
            .parse::<u16>()
            .map_err(|_| SdpError::Malformed(format!("invalid port: {s}")))?;
        Ok((port, 1))
    }
}

fn parse_attribute(value: &str) -> Result<SdpAttribute, SdpError> {
    if let Some((name, attr_value)) = value.split_once(':') {
        let name = name.trim();
        let attr_value = attr_value.trim();
        match name {
            "rtpmap" => parse_rtpmap(attr_value).map(SdpAttribute::RtpMap),
            "fmtp" => parse_fmtp(attr_value).map(|(pt, params)| SdpAttribute::Fmtp { pt, params }),
            "setup" => parse_setup(attr_value).map(SdpAttribute::Setup),
            "connection" => parse_connection_attr(attr_value).map(SdpAttribute::Connection),
            "ssrc" => parse_ssrc(attr_value).map(|(id, text)| SdpAttribute::Ssrc { id, text }),
            "y" => Ok(SdpAttribute::Y(attr_value.to_string())),
            "sendonly" => Ok(SdpAttribute::Direction(SdpDirection::SendOnly)),
            "recvonly" => Ok(SdpAttribute::Direction(SdpDirection::RecvOnly)),
            "sendrecv" => Ok(SdpAttribute::Direction(SdpDirection::SendRecv)),
            "inactive" => Ok(SdpAttribute::Direction(SdpDirection::Inactive)),
            other => Ok(SdpAttribute::Unknown {
                name: other.to_string(),
                value: if attr_value.is_empty() {
                    None
                } else {
                    Some(attr_value.to_string())
                },
            }),
        }
    } else {
        match value.trim() {
            "sendonly" => Ok(SdpAttribute::Direction(SdpDirection::SendOnly)),
            "recvonly" => Ok(SdpAttribute::Direction(SdpDirection::RecvOnly)),
            "sendrecv" => Ok(SdpAttribute::Direction(SdpDirection::SendRecv)),
            "inactive" => Ok(SdpAttribute::Direction(SdpDirection::Inactive)),
            other => Ok(SdpAttribute::Unknown {
                name: other.to_string(),
                value: None,
            }),
        }
    }
}

fn parse_rtpmap(value: &str) -> Result<RtpMap, SdpError> {
    let mut parts = value.splitn(2, char::is_whitespace);
    let pt = parts
        .next()
        .ok_or_else(|| SdpError::Malformed(format!("invalid rtpmap: {value}")))?
        .to_string();
    let rest = parts
        .next()
        .ok_or_else(|| SdpError::Malformed(format!("invalid rtpmap: {value}")))?;
    let mut encoding_parts = rest.split('/');
    let encoding = encoding_parts.next().unwrap_or("").to_string();
    let clock = encoding_parts.next().unwrap_or("").to_string();
    let params = encoding_parts.next().map(|s| s.to_string());
    Ok(RtpMap {
        pt,
        encoding,
        clock,
        params,
    })
}

fn parse_fmtp(value: &str) -> Result<(String, String), SdpError> {
    let mut parts = value.splitn(2, char::is_whitespace);
    let pt = parts
        .next()
        .ok_or_else(|| SdpError::Malformed(format!("invalid fmtp: {value}")))?
        .to_string();
    let params = parts.next().unwrap_or("").to_string();
    Ok((pt, params))
}

fn parse_setup(value: &str) -> Result<SdpSetup, SdpError> {
    match value.to_ascii_lowercase().as_str() {
        "active" => Ok(SdpSetup::Active),
        "passive" => Ok(SdpSetup::Passive),
        "actpass" => Ok(SdpSetup::Actpass),
        "none" => Ok(SdpSetup::None),
        other => Err(SdpError::Unsupported(format!("setup={other}"))),
    }
}

fn parse_connection_attr(value: &str) -> Result<SdpConnectionType, SdpError> {
    match value.to_ascii_lowercase().as_str() {
        "new" => Ok(SdpConnectionType::New),
        "existing" => Ok(SdpConnectionType::Existing),
        other => Err(SdpError::Unsupported(format!("connection={other}"))),
    }
}

fn parse_ssrc(value: &str) -> Result<(String, Option<String>), SdpError> {
    let mut parts = value.splitn(2, char::is_whitespace);
    let id = parts
        .next()
        .ok_or_else(|| SdpError::Malformed(format!("invalid ssrc: {value}")))?
        .to_string();
    let text = parts.next().map(|s| s.to_string());
    Ok((id, text))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parses_minimal_gb28181_sdp() {
        let body = "v=0\r\n\
                    o=- 0 0 IN IP4 0.0.0.0\r\n\
                    s=Play\r\n\
                    c=IN IP4 192.168.1.100\r\n\
                    t=0 0\r\n\
                    m=video 5000 TCP/RTP/AVP 96 98\r\n\
                    a=setup:passive\r\n\
                    a=connection:new\r\n\
                    a=rtpmap:96 PS/90000\r\n\
                    a=rtpmap:98 H264/90000\r\n\
                    a=y:0200000000";
        let session = parse_sdp(body.as_bytes(), &SdpParserConfig::default()).unwrap();
        assert_eq!(session.version, "0");
        assert_eq!(session.origin.username, "-");
        assert_eq!(session.name, "Play");
        assert_eq!(session.media.len(), 1);

        let media = &session.media[0];
        assert_eq!(media.media_type, "video");
        assert_eq!(media.port, 5000);
        assert_eq!(media.proto, "TCP/RTP/AVP");
        assert_eq!(media.formats, vec!["96", "98"]);
        assert_eq!(media.setup(), Some(SdpSetup::Passive));
        assert_eq!(media.connection_attr(), Some(SdpConnectionType::New));
        assert_eq!(media.y_ssrc(), Some("0200000000"));

        let rtpmap = media.rtpmap_for("96").unwrap();
        assert_eq!(rtpmap.encoding, "PS");
        assert_eq!(rtpmap.clock, "90000");
    }

    #[test]
    fn rejects_excessive_lines() {
        use std::fmt::Write as _;
        let mut body = String::new();
        write!(&mut body, "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=Play\r\n").unwrap();
        for i in 0..SdpParserConfig::test().max_lines + 1 {
            write!(&mut body, "a=x:{i}\r\n").unwrap();
        }
        assert!(parse_sdp(body.as_bytes(), &SdpParserConfig::test()).is_err());
    }
}

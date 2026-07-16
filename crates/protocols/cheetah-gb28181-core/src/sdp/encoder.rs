//! SDP encoder using `std::fmt::Write` to avoid ad-hoc string concatenation.

use super::session::{
    RtpMap, SdpAttribute, SdpConnectionType, SdpDirection, SdpMedia, SdpSession, SdpSetup,
};
use std::fmt::Write as _;

/// Encodes an SDP session to a `CRLF`-terminated string.
pub fn encode_sdp(session: &SdpSession) -> Result<String, SdpError> {
    validate_session(session)?;
    let mut out = String::new();
    write_session(&mut out, session)?;
    for time in &session.times {
        write!(&mut out, "t={} {}\r\n", time.start, time.stop)
            .map_err(|e| SdpError::Malformed(e.to_string()))?;
    }
    for media in &session.media {
        write_media(&mut out, media)?;
    }
    Ok(out)
}

fn validate_no_crlf(value: &str) -> Result<(), SdpError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(SdpError::Malformed(format!(
            "SDP field contains forbidden line break: {value:?}"
        )));
    }
    Ok(())
}

fn validate_session(session: &SdpSession) -> Result<(), SdpError> {
    validate_no_crlf(&session.version)?;
    validate_no_crlf(&session.origin.username)?;
    validate_no_crlf(&session.origin.sess_id)?;
    validate_no_crlf(&session.origin.sess_version)?;
    validate_no_crlf(&session.origin.nettype)?;
    validate_no_crlf(&session.origin.addrtype)?;
    validate_no_crlf(&session.origin.address)?;
    validate_no_crlf(&session.name)?;
    if let Some(info) = &session.info {
        validate_no_crlf(info)?;
    }
    if let Some(subject) = &session.subject {
        validate_no_crlf(subject)?;
    }
    if let Some(conn) = &session.connection {
        validate_no_crlf(&conn.nettype)?;
        validate_no_crlf(&conn.addrtype)?;
        validate_no_crlf(&conn.address)?;
    }
    for attr in &session.attributes {
        validate_attribute(attr)?;
    }
    for time in &session.times {
        validate_no_crlf(&time.start)?;
        validate_no_crlf(&time.stop)?;
    }
    for media in &session.media {
        validate_media(media)?;
    }
    Ok(())
}

fn validate_media(media: &SdpMedia) -> Result<(), SdpError> {
    validate_no_crlf(&media.media_type)?;
    validate_no_crlf(&media.proto)?;
    for fmt in &media.formats {
        validate_no_crlf(fmt)?;
    }
    if let Some(title) = &media.title {
        validate_no_crlf(title)?;
    }
    if let Some(conn) = &media.connection {
        validate_no_crlf(&conn.nettype)?;
        validate_no_crlf(&conn.addrtype)?;
        validate_no_crlf(&conn.address)?;
    }
    for attr in &media.attributes {
        validate_attribute(attr)?;
    }
    Ok(())
}

fn validate_attribute(attr: &SdpAttribute) -> Result<(), SdpError> {
    match attr {
        SdpAttribute::RtpMap(RtpMap {
            pt,
            encoding,
            clock,
            params,
        }) => {
            validate_no_crlf(pt)?;
            validate_no_crlf(encoding)?;
            validate_no_crlf(clock)?;
            if let Some(p) = params {
                validate_no_crlf(p)?;
            }
        }
        SdpAttribute::Fmtp { pt, params } => {
            validate_no_crlf(pt)?;
            validate_no_crlf(params)?;
        }
        SdpAttribute::Ssrc { id, text } => {
            validate_no_crlf(id)?;
            if let Some(t) = text {
                validate_no_crlf(t)?;
            }
        }
        SdpAttribute::Y(v) => validate_no_crlf(v)?,
        SdpAttribute::Unknown { name, value } => {
            validate_no_crlf(name)?;
            if let Some(v) = value {
                validate_no_crlf(v)?;
            }
        }
        SdpAttribute::Setup(_) | SdpAttribute::Connection(_) | SdpAttribute::Direction(_) => {}
    }
    Ok(())
}

use super::error::SdpError;

fn write_session(out: &mut String, session: &SdpSession) -> Result<(), SdpError> {
    write!(out, "v={}\r\n", session.version).map_err(|e| SdpError::Malformed(e.to_string()))?;
    write!(
        out,
        "o={} {} {} {} {} {}\r\n",
        session.origin.username,
        session.origin.sess_id,
        session.origin.sess_version,
        session.origin.nettype,
        session.origin.addrtype,
        session.origin.address
    )
    .map_err(|e| SdpError::Malformed(e.to_string()))?;
    write!(out, "s={}\r\n", session.name).map_err(|e| SdpError::Malformed(e.to_string()))?;
    if let Some(info) = &session.info {
        write!(out, "i={}\r\n", info).map_err(|e| SdpError::Malformed(e.to_string()))?;
    }
    if let Some(conn) = &session.connection {
        write!(
            out,
            "c={} {} {}\r\n",
            conn.nettype, conn.addrtype, conn.address
        )
        .map_err(|e| SdpError::Malformed(e.to_string()))?;
    }
    for attr in &session.attributes {
        write_attribute(out, attr)?;
    }
    Ok(())
}

fn write_media(out: &mut String, media: &SdpMedia) -> Result<(), SdpError> {
    write!(out, "m={} ", media.media_type).map_err(|e| SdpError::Malformed(e.to_string()))?;
    if media.port_count > 1 {
        write!(out, "{}/{}", media.port, media.port_count)
            .map_err(|e| SdpError::Malformed(e.to_string()))?;
    } else {
        write!(out, "{}", media.port).map_err(|e| SdpError::Malformed(e.to_string()))?;
    }
    write!(out, " {}", media.proto).map_err(|e| SdpError::Malformed(e.to_string()))?;
    for fmt in &media.formats {
        write!(out, " {fmt}").map_err(|e| SdpError::Malformed(e.to_string()))?;
    }
    write!(out, "\r\n").map_err(|e| SdpError::Malformed(e.to_string()))?;

    if let Some(title) = &media.title {
        write!(out, "i={}\r\n", title).map_err(|e| SdpError::Malformed(e.to_string()))?;
    }
    if let Some(conn) = &media.connection {
        write!(
            out,
            "c={} {} {}\r\n",
            conn.nettype, conn.addrtype, conn.address
        )
        .map_err(|e| SdpError::Malformed(e.to_string()))?;
    }
    for attr in &media.attributes {
        write_attribute(out, attr)?;
    }
    Ok(())
}

fn write_attribute(out: &mut String, attr: &SdpAttribute) -> Result<(), SdpError> {
    match attr {
        SdpAttribute::RtpMap(RtpMap {
            pt,
            encoding,
            clock,
            params,
        }) => if let Some(params) = params {
            write!(out, "a=rtpmap:{pt} {encoding}/{clock}/{params}\r\n")
        } else {
            write!(out, "a=rtpmap:{pt} {encoding}/{clock}\r\n")
        }
        .map_err(|e| SdpError::Malformed(e.to_string())),
        SdpAttribute::Fmtp { pt, params } => {
            write!(out, "a=fmtp:{pt} {params}\r\n").map_err(|e| SdpError::Malformed(e.to_string()))
        }
        SdpAttribute::Setup(setup) => {
            let s = match setup {
                SdpSetup::Active => "active",
                SdpSetup::Passive => "passive",
                SdpSetup::Actpass => "actpass",
                SdpSetup::None => "none",
            };
            write!(out, "a=setup:{s}\r\n").map_err(|e| SdpError::Malformed(e.to_string()))
        }
        SdpAttribute::Connection(conn) => {
            let s = match conn {
                SdpConnectionType::New => "new",
                SdpConnectionType::Existing => "existing",
            };
            write!(out, "a=connection:{s}\r\n").map_err(|e| SdpError::Malformed(e.to_string()))
        }
        SdpAttribute::Ssrc { id, text } => if let Some(text) = text {
            write!(out, "a=ssrc:{id} {text}\r\n")
        } else {
            write!(out, "a=ssrc:{id}\r\n")
        }
        .map_err(|e| SdpError::Malformed(e.to_string())),
        SdpAttribute::Y(v) => {
            write!(out, "a=y:{v}\r\n").map_err(|e| SdpError::Malformed(e.to_string()))
        }
        SdpAttribute::Direction(d) => {
            let s = match d {
                SdpDirection::SendOnly => "sendonly",
                SdpDirection::RecvOnly => "recvonly",
                SdpDirection::SendRecv => "sendrecv",
                SdpDirection::Inactive => "inactive",
            };
            write!(out, "a={s}\r\n").map_err(|e| SdpError::Malformed(e.to_string()))
        }
        SdpAttribute::Unknown { name, value } => if let Some(value) = value {
            write!(out, "a={name}:{value}\r\n")
        } else {
            write!(out, "a={name}\r\n")
        }
        .map_err(|e| SdpError::Malformed(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::super::parser::parse_sdp;
    use super::super::session::{SdpConnection, SdpOrigin, SdpSession, SdpTime};
    use super::*;

    #[test]
    fn round_trip_minimal_sdp() {
        let session = SdpSession {
            version: "0".to_string(),
            origin: SdpOrigin {
                username: "-".to_string(),
                sess_id: "0".to_string(),
                sess_version: "0".to_string(),
                nettype: "IN".to_string(),
                addrtype: "IP4".to_string(),
                address: "0.0.0.0".to_string(),
            },
            name: "Play".to_string(),
            connection: Some(SdpConnection {
                nettype: "IN".to_string(),
                addrtype: "IP4".to_string(),
                address: "192.168.1.100".to_string(),
            }),
            times: vec![SdpTime {
                start: "0".to_string(),
                stop: "0".to_string(),
            }],
            ..Default::default()
        };
        let encoded = encode_sdp(&session).unwrap();
        assert!(encoded.contains("v=0\r\n"));
        assert!(encoded.contains("o=- 0 0 IN IP4 0.0.0.0\r\n"));
        assert!(encoded.contains("c=IN IP4 192.168.1.100\r\n"));
        let reparsed = parse_sdp(
            encoded.as_bytes(),
            &super::super::parser::SdpParserConfig::default(),
        )
        .unwrap();
        assert_eq!(reparsed, session);
    }
}

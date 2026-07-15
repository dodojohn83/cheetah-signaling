//! GB28181 XML (MANSCDP / MANSRTSP) parsing helpers.

use crate::error::AccessError;
use quick_xml::Reader;
use quick_xml::events::Event;

/// Parsed content of a GB28181 `Keepalive` NOTIFY message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeepaliveInfo {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Status string from the `<Status>` element (typically `OK`).
    pub status: String,
}

/// Default safety limits for XML parsing. These protect against malformed or
/// adversarial bodies before the codec is fully hardened in GB-ACC-003.
const MAX_DEPTH: usize = 32;
const MAX_TEXT_LEN: usize = 4096;
const MAX_EVENTS: usize = 1024;
const MAX_BODY_BYTES: usize = 65536;

/// Parses a `Keepalive` message body and returns the extracted fields.
///
/// The body must be a valid XML document with a root element (typically
/// `<Notify>`) containing `<CmdType>Keepalive</CmdType>`, `<SN>`,
/// `<DeviceID>` and `<Status>` children.
///
/// Returns `AccessError::InvalidXml` for non-Keepalive messages or bodies that
/// exceed safety limits.
pub fn parse_keepalive(body: &[u8]) -> Result<KeepaliveInfo, AccessError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(AccessError::InvalidXml("body too large".to_string()));
    }

    let mut reader = Reader::from_reader(body);
    let mut stack: Vec<String> = Vec::with_capacity(8);
    let mut cmd_type: Option<String> = None;
    let mut sn: Option<String> = None;
    let mut device_id: Option<String> = None;
    let mut status: Option<String> = None;
    let mut current_text = String::new();
    let mut current_path = String::new();
    let mut event_count = 0usize;

    loop {
        if event_count > MAX_EVENTS {
            return Err(AccessError::InvalidXml("too many XML events".to_string()));
        }
        event_count += 1;

        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = reader
                    .decoder()
                    .decode(e.name().as_ref())
                    .map_err(|e| AccessError::InvalidXml(e.to_string()))?
                    .into_owned();
                if stack.len() >= MAX_DEPTH {
                    return Err(AccessError::InvalidXml("XML nesting too deep".to_string()));
                }
                stack.push(name);
                current_text.clear();
            }
            Ok(Event::Text(e)) => {
                let text = reader
                    .decoder()
                    .decode(e.as_ref())
                    .map_err(|e| AccessError::InvalidXml(e.to_string()))?;
                // Trim must not allocate unbounded amounts; `MAX_TEXT_LEN` is
                // enforced after accumulation below.
                current_text.push_str(text.trim());
                if current_text.len() > MAX_TEXT_LEN {
                    return Err(AccessError::InvalidXml("text node too long".to_string()));
                }
            }
            Ok(Event::End(e)) => {
                let name = reader
                    .decoder()
                    .decode(e.name().as_ref())
                    .map_err(|e| AccessError::InvalidXml(e.to_string()))?
                    .into_owned();
                if stack.pop().as_deref() != Some(name.as_str()) {
                    return Err(AccessError::InvalidXml("mismatched XML tags".to_string()));
                }

                build_path(&stack, &name, &mut current_path);
                if stack.len() == 1 {
                    capture_leaf(
                        &current_path,
                        &mut current_text,
                        &mut cmd_type,
                        &mut sn,
                        &mut device_id,
                        &mut status,
                    );
                }
            }
            Ok(Event::Empty(e)) => {
                let name = reader
                    .decoder()
                    .decode(e.name().as_ref())
                    .map_err(|e| AccessError::InvalidXml(e.to_string()))?
                    .into_owned();
                if stack.len() + 1 > MAX_DEPTH {
                    return Err(AccessError::InvalidXml("XML nesting too deep".to_string()));
                }

                build_path(&stack, &name, &mut current_path);
                if stack.len() == 1 {
                    capture_leaf(
                        &current_path,
                        &mut current_text,
                        &mut cmd_type,
                        &mut sn,
                        &mut device_id,
                        &mut status,
                    );
                }
            }
            Ok(Event::DocType(_)) => {
                return Err(AccessError::InvalidXml("DOCTYPE not allowed".to_string()));
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(AccessError::InvalidXml(e.to_string())),
            _ => {}
        }
    }

    if cmd_type.as_deref() != Some("Keepalive") {
        return Err(AccessError::UnsupportedCmdType(
            cmd_type.unwrap_or_default(),
        ));
    }

    Ok(KeepaliveInfo {
        sn: sn.unwrap_or_default(),
        device_id: device_id.unwrap_or_default(),
        status: status.unwrap_or_default(),
    })
}

fn build_path(stack: &[String], leaf: &str, path: &mut String) {
    path.clear();
    for (i, part) in stack.iter().enumerate() {
        if i > 0 {
            path.push('/');
        }
        path.push_str(part);
    }
    if !path.is_empty() {
        path.push('/');
    }
    path.push_str(leaf);
}

fn capture_leaf(
    path: &str,
    current_text: &mut String,
    cmd_type: &mut Option<String>,
    sn: &mut Option<String>,
    device_id: &mut Option<String>,
    status: &mut Option<String>,
) {
    match path {
        _ if path.ends_with("/CmdType") => {
            *cmd_type = Some(std::mem::take(current_text));
        }
        _ if path.ends_with("/SN") => {
            *sn = Some(std::mem::take(current_text));
        }
        _ if path.ends_with("/DeviceID") => {
            *device_id = Some(std::mem::take(current_text));
        }
        _ if path.ends_with("/Status") => {
            *status = Some(std::mem::take(current_text));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_valid_keepalive() {
        let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Status>OK</Status>
</Notify>"#;
        let info = parse_keepalive(body).unwrap();
        assert_eq!(info.sn, "1");
        assert_eq!(info.device_id, "34020000001320000001");
        assert_eq!(info.status, "OK");
    }

    #[test]
    fn rejects_non_keepalive() {
        let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Catalog</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
</Notify>"#;
        assert!(parse_keepalive(body).is_err());
    }

    #[test]
    fn rejects_doctype() {
        let body = br#"<?xml version="1.0"?>
<!DOCTYPE foo SYSTEM "http://example.com/foo.dtd">
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Status>OK</Status>
</Notify>"#;
        assert!(parse_keepalive(body).is_err());
    }
}

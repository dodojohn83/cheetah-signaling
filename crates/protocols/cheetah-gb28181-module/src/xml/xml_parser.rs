//! Streaming XML parser with charset handling and resource limits.

use crate::{Gb28181Config, Gb28181ModuleError, config::CharsetPolicy};
use quick_xml::Reader;
use quick_xml::events::Event;

use super::xml_model::Gb28181Envelope;

/// Parses a GB28181 XML body with configured limits and charset handling.
///
/// Rejects bodies exceeding the configured size, `DOCTYPE`/`ENTITY` declarations
/// and unsupported encodings.
pub fn parse_xml(
    bytes: &[u8],
    config: &Gb28181Config,
) -> Result<Gb28181Envelope, Gb28181ModuleError> {
    if bytes.len() > config.xml_limits.max_body_bytes {
        return Err(Gb28181ModuleError::Xml(format!(
            "XML body exceeds {} bytes",
            config.xml_limits.max_body_bytes
        )));
    }

    let text = decode_xml_body(bytes, &config.charset_policy)?;
    validate_xml_limits(&text, &config.xml_limits)?;

    let envelope: Gb28181Envelope = quick_xml::de::from_str(&text)
        .map_err(|e| Gb28181ModuleError::Xml(format!("parse error: {e}")))?;
    Ok(envelope)
}

fn decode_xml_body(bytes: &[u8], _policy: &CharsetPolicy) -> Result<String, Gb28181ModuleError> {
    let declaration = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    let lower = declaration.to_ascii_lowercase();
    let encoding = if let Some((_, rest)) = lower.split_once("encoding=\"") {
        rest.split(['"', '\'']).next().unwrap_or("utf-8")
    } else if let Some((_, rest)) = lower.split_once("encoding='") {
        rest.split(['"', '\'']).next().unwrap_or("utf-8")
    } else {
        "utf-8"
    };

    if encoding.eq_ignore_ascii_case("utf-8") || encoding.eq_ignore_ascii_case("us-ascii") {
        return String::from_utf8(bytes.to_vec())
            .map_err(|e| Gb28181ModuleError::Xml(format!("invalid UTF-8: {e}")));
    }

    if encoding.eq_ignore_ascii_case("gb2312") || encoding.eq_ignore_ascii_case("gbk") {
        let (cow, _, had_errors) = encoding_rs::GBK.decode(bytes);
        if had_errors {
            return Err(Gb28181ModuleError::Xml(format!(
                "failed to decode {encoding} body"
            )));
        }
        return Ok(cow.into_owned());
    }

    Err(Gb28181ModuleError::Xml(format!(
        "unsupported XML encoding: {encoding}"
    )))
}

fn validate_xml_limits(
    text: &str,
    limits: &crate::config::XmlLimits,
) -> Result<(), Gb28181ModuleError> {
    if text.len() > limits.max_body_bytes {
        return Err(Gb28181ModuleError::Xml(format!(
            "decoded XML body exceeds {} bytes",
            limits.max_body_bytes
        )));
    }

    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    reader.config_mut().trim_text(true);

    let mut depth: usize = 0;
    let mut max_depth: usize = 0;
    let mut item_count: usize = 0;
    let mut buf = Vec::new();

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| Gb28181ModuleError::Xml(format!("XML validation error: {e}")))?;
        match event {
            Event::Start(e) | Event::Empty(e) => {
                depth += 1;
                max_depth = max_depth.max(depth);
                if max_depth > limits.max_depth {
                    return Err(Gb28181ModuleError::Xml(format!(
                        "XML depth exceeds {}",
                        limits.max_depth
                    )));
                }
                if e.attributes().count() > limits.max_attrs_per_element {
                    return Err(Gb28181ModuleError::Xml(
                        "too many attributes on element".to_string(),
                    ));
                }
                if e.name().local_name().as_ref() == b"Item" {
                    item_count += 1;
                    if item_count > limits.max_list_items {
                        return Err(Gb28181ModuleError::Xml(format!(
                            "too many list items: limit {}",
                            limits.max_list_items
                        )));
                    }
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
            }
            Event::Text(e) => {
                let len = e.decode().map(|s| s.len()).unwrap_or(e.len());
                if len > limits.max_text_len {
                    return Err(Gb28181ModuleError::Xml(format!(
                        "text node exceeds {} bytes",
                        limits.max_text_len
                    )));
                }
            }
            Event::DocType(_) => {
                return Err(Gb28181ModuleError::Xml(
                    "XML DTD/ENTITY declarations are not allowed".into(),
                ));
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(())
}

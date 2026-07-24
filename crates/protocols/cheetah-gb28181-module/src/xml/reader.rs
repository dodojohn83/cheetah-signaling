//! Streaming XML reader for GB28181 MANSCDP/MANSRTSP payloads.

use super::element::XmlElement;
use super::limits::XmlLimits;
use crate::error::AccessError;
use cheetah_domain::{CompatibilityCapability, CompatibilityProfile};
use encoding_rs::{Encoding, GB18030, GBK, UTF_8};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// Parses a raw XML body into a generic element tree using the default
/// (strict) compatibility profile.
pub fn parse_xml(body: &[u8], limits: &XmlLimits) -> Result<XmlElement, AccessError> {
    parse_xml_with_profile(body, limits, &CompatibilityProfile::default())
}

/// Parses a raw XML body into a generic element tree under a compatibility
/// profile.
///
/// The body is decoded according to the `encoding` attribute in the XML
/// declaration, or UTF-8 by default. GB2312/GBK payloads are decoded using
/// `encoding_rs`. With [`CompatibilityCapability::CharsetFallback`] enabled,
/// a declared-UTF-8 byte stream that fails UTF-8 validation is re-attempted
/// with GBK and GB18030 before rejecting.
pub fn parse_xml_with_profile(
    body: &[u8],
    limits: &XmlLimits,
    profile: &CompatibilityProfile,
) -> Result<XmlElement, AccessError> {
    if body.len() > limits.max_body_bytes {
        return Err(AccessError::invalid_xml("body too large"));
    }

    let decoded = decode_body(body, profile)?;

    let mut reader = Reader::from_reader(decoded.as_bytes());
    reader.config_mut().trim_text(true);

    let mut stack: Vec<XmlElement> = Vec::with_capacity(8);
    let mut root: Option<XmlElement> = None;
    let mut event_count = 0usize;
    let mut total_attributes = 0usize;

    loop {
        if event_count > limits.max_events {
            return Err(AccessError::invalid_xml("too many XML events"));
        }
        event_count += 1;

        match reader.read_event() {
            Ok(Event::Decl(_)) => {}
            Ok(Event::PI(_)) => {}
            Ok(Event::Comment(_)) => {}
            Ok(Event::Start(e)) => {
                if stack.len() >= limits.max_depth {
                    return Err(AccessError::invalid_xml("XML nesting too deep"));
                }
                let element = start_element(&e, &reader, limits, &mut total_attributes)?;
                stack.push(element);
            }
            Ok(Event::Empty(e)) => {
                let element = start_element(&e, &reader, limits, &mut total_attributes)?;
                if let Some(parent) = stack.last_mut() {
                    if parent.children.len() >= limits.max_children_per_element {
                        return Err(AccessError::invalid_xml("too many children per element"));
                    }
                    parent.children.push(element);
                } else {
                    if root.is_some() {
                        return Err(AccessError::invalid_xml("multiple top-level elements"));
                    }
                    root = Some(element);
                }
            }
            Ok(Event::Text(e)) => {
                if let Some(top) = stack.last_mut() {
                    let text = e
                        .decode()
                        .map_err(|_| AccessError::invalid_xml("bad entity"))?;
                    top.text.push_str(text.trim());
                    if top.text.len() > limits.max_text_len {
                        return Err(AccessError::invalid_xml("text node too long"));
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = decode_name(e.name().local_name().as_ref(), &reader)?;
                let mut top = stack
                    .pop()
                    .ok_or_else(|| AccessError::invalid_xml("unexpected end tag"))?;
                if top.name != name {
                    return Err(AccessError::invalid_xml("mismatched XML tags"));
                }
                top.text = top.text.trim().to_string();
                if let Some(parent) = stack.last_mut() {
                    if parent.children.len() >= limits.max_children_per_element {
                        return Err(AccessError::invalid_xml("too many children per element"));
                    }
                    parent.children.push(top);
                } else {
                    if root.is_some() {
                        return Err(AccessError::invalid_xml("multiple top-level elements"));
                    }
                    root = Some(top);
                }
            }
            Ok(Event::DocType(_)) => {
                return Err(AccessError::invalid_xml("DOCTYPE not allowed"));
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(AccessError::invalid_xml(e)),
            _ => {}
        }
    }

    root.ok_or_else(|| AccessError::invalid_xml("empty XML document"))
}

fn decode_body<'a>(
    body: &'a [u8],
    profile: &CompatibilityProfile,
) -> Result<std::borrow::Cow<'a, str>, AccessError> {
    let declared = detect_encoding(body).unwrap_or(UTF_8);
    let (decoded, had_errors) = declared.decode_without_bom_handling(body);
    if !had_errors {
        return Ok(decoded);
    }
    if profile.has(CompatibilityCapability::CharsetFallback) {
        for fallback in [GBK, GB18030, UTF_8] {
            if fallback == declared {
                continue;
            }
            let (d, err) = fallback.decode_without_bom_handling(body);
            if !err {
                return Ok(d);
            }
        }
    }
    Err(AccessError::invalid_xml(
        "invalid byte sequence for declared encoding",
    ))
}

fn start_element(
    e: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    limits: &XmlLimits,
    total_attributes: &mut usize,
) -> Result<XmlElement, AccessError> {
    let name = decode_name(e.name().local_name().as_ref(), reader)?;
    validate_len(&name, limits.max_name_len, "element name too long")?;

    let mut element = XmlElement {
        name,
        ..XmlElement::default()
    };

    let mut attr_count = 0usize;
    for attr in e.attributes() {
        let attr = attr.map_err(|_| AccessError::invalid_xml("bad attribute"))?;
        let key = decode_name(attr.key.local_name().as_ref(), reader)?;
        validate_len(&key, limits.max_name_len, "attribute name too long")?;
        let value = reader
            .decoder()
            .decode(attr.value.as_ref())
            .map_err(|_| AccessError::invalid_xml("invalid attribute value"))?
            .into_owned();
        attr_count += 1;
        *total_attributes += 1;
        if attr_count > limits.max_attributes_per_element {
            return Err(AccessError::invalid_xml("too many attributes on element"));
        }
        if *total_attributes > limits.max_total_attributes {
            return Err(AccessError::invalid_xml("too many attributes in document"));
        }
        validate_len(&value, limits.max_text_len, "attribute value too long")?;
        element.attributes.insert(key, value);
    }

    Ok(element)
}

fn decode_name(name: &[u8], reader: &Reader<&[u8]>) -> Result<String, AccessError> {
    reader
        .decoder()
        .decode(name)
        .map_err(|_| AccessError::invalid_xml("invalid name"))
        .map(|s| s.into_owned())
}

fn validate_len(s: &str, max: usize, msg: &str) -> Result<(), AccessError> {
    if s.len() > max {
        return Err(AccessError::invalid_xml(msg));
    }
    Ok(())
}

fn detect_encoding(body: &[u8]) -> Option<&'static Encoding> {
    let prefix_len = body.len().min(256);
    let prefix = String::from_utf8_lossy(&body[..prefix_len]);
    let decl_start = prefix.find("<?xml")?;
    let decl_end = prefix[decl_start..].find("?>")? + decl_start;
    let decl = &prefix[decl_start..decl_end];

    let idx = decl.find("encoding")?;
    let mut after = &decl[idx + "encoding".len()..];
    after = after.trim_start();
    if !after.starts_with('=') {
        return None;
    }
    after = after[1..].trim_start();
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after[1..];
    let close = rest.find(quote)?;
    let label = &rest[..close];
    Encoding::for_label(label.as_bytes())
}

//! Type-safe XML writer for GB28181 MANSCDP/MANSRTSP payloads.

use super::element::XmlElement;
use super::limits::XmlLimits;
use crate::error::AccessError;

/// Encodes a generic element tree as a UTF-8 XML string with the optional
/// declaration.
///
/// Text and attribute values are escaped; string concatenation is not used to
/// build the document.
pub fn encode_xml(element: &XmlElement, declaration: bool) -> Result<String, AccessError> {
    let mut output = String::new();
    if declaration {
        output.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    }
    write_element(&mut output, element, &XmlLimits::default())?;
    Ok(output)
}

fn write_element(
    out: &mut String,
    element: &XmlElement,
    limits: &XmlLimits,
) -> Result<(), AccessError> {
    validate_xml_name(&element.name)?;
    out.push('<');
    out.push_str(&element.name);

    for (key, value) in &element.attributes {
        validate_xml_name(key)?;
        out.push(' ');
        out.push_str(key);
        out.push_str("=\"");
        out.push_str(&escape_attr(value));
        out.push('"');
    }

    let has_text = !element.text.is_empty();
    let has_children = !element.children.is_empty();

    if !has_text && !has_children {
        out.push_str("/>");
        return Ok(());
    }

    out.push('>');

    if has_text {
        if element.text.len() > limits.max_text_len {
            return Err(AccessError::InvalidXml("text node too long".to_string()));
        }
        out.push_str(&escape_text(&element.text));
    }

    for child in &element.children {
        write_element(out, child, limits)?;
    }

    out.push_str("</");
    out.push_str(&element.name);
    out.push('>');
    Ok(())
}

fn escape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn validate_xml_name(name: &str) -> Result<(), AccessError> {
    if name.is_empty() {
        return Err(AccessError::InvalidXml("empty element name".to_string()));
    }
    // GB28181 element names are ASCII identifiers.
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AccessError::InvalidXml(
            "invalid character in XML name".to_string(),
        ));
    }
    Ok(())
}

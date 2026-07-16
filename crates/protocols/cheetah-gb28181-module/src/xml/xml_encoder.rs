//! Type-safe GB28181 XML request encoder.

use crate::Gb28181ModuleError;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

/// Generates a GB28181 XML request body.
///
/// The implementation uses `quick-xml`'s writer so all element text is escaped.
pub fn encode_request(
    root: &str,
    cmd_type: &str,
    sn: u32,
    device_id: &str,
    extra: &[(&str, &str)],
) -> Result<String, Gb28181ModuleError> {
    let mut writer = Writer::new(Vec::new());
    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;
    writer
        .write_event(Event::Start(BytesStart::new(root)))
        .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;

    let sn_str = sn.to_string();
    for (tag, value) in [
        ("CmdType", cmd_type.as_bytes()),
        ("SN", sn_str.as_bytes()),
        ("DeviceID", device_id.as_bytes()),
    ] {
        writer
            .write_event(Event::Start(BytesStart::new(tag)))
            .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;
        writer
            .write_event(Event::Text(BytesText::new(
                std::str::from_utf8(value).unwrap_or(""),
            )))
            .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;
        writer
            .write_event(Event::End(BytesEnd::new(tag)))
            .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;
    }

    for &(k, v) in extra {
        writer
            .write_event(Event::Start(BytesStart::new(k)))
            .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;
        writer
            .write_event(Event::Text(BytesText::new(v)))
            .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;
        writer
            .write_event(Event::End(BytesEnd::new(k)))
            .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;
    }

    writer
        .write_event(Event::End(BytesEnd::new(root)))
        .map_err(|e| Gb28181ModuleError::Xml(e.to_string()))?;

    String::from_utf8(writer.into_inner())
        .map_err(|e| Gb28181ModuleError::Xml(format!("invalid UTF-8: {e}")))
}

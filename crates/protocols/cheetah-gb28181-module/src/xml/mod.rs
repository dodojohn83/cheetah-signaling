//! GB28181 XML (MANSCDP / MANSRTSP) codec.

pub mod alarm;
pub mod catalog;
pub mod device_control;
pub mod device_info;
pub mod device_status;
pub mod element;
pub mod keepalive;
pub mod limits;
pub mod mobile_position;
pub mod reader;
pub mod record_info;
pub mod writer;

pub use alarm::{AlarmInfo, build_alarm_notify, parse_alarm};
pub use catalog::{
    CatalogItem, CatalogQuery, CatalogResponse, build_catalog_response, parse_catalog,
    parse_catalog_query,
};
pub use device_control::{
    DeviceControlKind, DeviceControlRequest, DeviceControlResponse, PresetAction, PresetCommand,
    PtzCommand, parse_device_control_response,
};
pub use device_info::{DeviceInfoResponse, parse_device_info};
pub use device_status::{DeviceStatusResponse, build_device_status_notify, parse_device_status};
pub use element::XmlElement;
pub use keepalive::{
    KeepaliveInfo, KeepaliveResponse, build_keepalive, parse_keepalive, parse_keepalive_response,
};
pub use limits::XmlLimits;
pub use mobile_position::{
    MobilePositionInfo, build_mobile_position_notify, parse_mobile_position,
};
pub use reader::parse_xml;
pub use record_info::{RecordInfoResponse, RecordItem, parse_record_info};
pub use writer::encode_xml;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::AccessError;

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
        assert!(info.extensions.is_empty());
    }

    #[test]
    fn rejects_non_keepalive() {
        let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Catalog</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
</Notify>"#;
        let err = parse_keepalive(body).unwrap_err();
        assert!(matches!(err, AccessError::UnsupportedCmdType(_)));
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

    #[test]
    fn round_trip_xml() {
        let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Status>OK</Status>
</Notify>"#;
        let root = reader::parse_xml(body, &XmlLimits::default()).unwrap();
        let encoded = writer::encode_xml(&root, true).unwrap();
        let reparsed = reader::parse_xml(encoded.as_bytes(), &XmlLimits::default()).unwrap();
        assert_eq!(root, reparsed);
    }

    #[test]
    fn escapes_text_and_attributes() {
        let mut root = element::XmlElement {
            name: "Notify".to_string(),
            ..element::XmlElement::default()
        };
        let mut child = element::XmlElement {
            name: "Status".to_string(),
            text: "A&B<C>D\"E'F".to_string(),
            ..element::XmlElement::default()
        };
        child
            .attributes
            .insert("val".to_string(), "x&y".to_string());
        root.children.push(child);

        let encoded = writer::encode_xml(&root, false).unwrap();
        // Text must escape &, < and > but not quotes or apostrophes.
        assert!(encoded.contains("A&amp;B&lt;C&gt;D\"E'F"));
        // Attributes must escape &, <, >, quotes and apostrophes.
        assert!(encoded.contains(r#"val="x&amp;y""#));
        assert!(!encoded.contains("&C>"));
    }

    #[test]
    fn rejects_too_deep() {
        let mut body = String::new();
        for _ in 0..XmlLimits::test().max_depth + 1 {
            body.push_str("<a>");
        }
        body.push('x');
        for _ in 0..XmlLimits::test().max_depth + 1 {
            body.push_str("</a>");
        }
        assert!(reader::parse_xml(body.as_bytes(), &XmlLimits::test()).is_err());
    }

    #[test]
    fn rejects_too_many_children() {
        let mut body = String::from("<root>");
        for i in 0..XmlLimits::test().max_children_per_element + 1 {
            body.push_str(&format!("<c>{i}</c>"));
        }
        body.push_str("</root>");
        assert!(reader::parse_xml(body.as_bytes(), &XmlLimits::test()).is_err());
    }

    #[test]
    fn parses_gbk_encoded_body() {
        // "你好" encoded as GBK.
        let text = encoding_rs::GBK
            .encode("<Notify><Status>你好</Status></Notify>")
            .0;
        let mut body = b"<?xml version=\"1.0\" encoding=\"GBK\"?>\n".to_vec();
        body.extend_from_slice(&text);

        let root = reader::parse_xml(&body, &XmlLimits::default()).unwrap();
        assert_eq!(root.child_text("Status").unwrap(), "你好");
    }

    #[test]
    fn preserves_unknown_extensions() {
        let body = br#"<?xml version="1.0"?>
<Notify>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Status>OK</Status>
    <Extra>value</Extra>
</Notify>"#;
        let info = parse_keepalive(body).unwrap();
        assert_eq!(
            info.extensions.get("Extra").map(String::as_str),
            Some("value")
        );
    }

    #[test]
    fn parse_valid_keepalive_response() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>OK</Result>
</Response>"#;
        let resp = parse_keepalive_response(body).unwrap();
        assert_eq!(resp.sn, "1");
        assert_eq!(resp.device_id, "34020000001320000001");
        assert_eq!(resp.result, "OK");
    }

    #[test]
    fn parse_keepalive_response_rejects_error_result() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>Keepalive</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Result>ERROR</Result>
</Response>"#;
        let resp = parse_keepalive_response(body).unwrap();
        assert_eq!(resp.result, "ERROR");
    }
}

//! GB28181 XML codec with size, depth, and entity limits.
//!
//! This module re-exports the model, streaming parser, and type-safe encoder.

mod xml_encoder;
mod xml_model;
mod xml_parser;

pub use xml_encoder::encode_request;
pub use xml_model::{Gb28181Envelope, Gb28181Message, Item, ItemList, RecordItem, RecordList};
pub use xml_parser::parse_xml;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Gb28181Config;
    use crate::config::Gb28181ConfigBuilder;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    fn test_config() -> Gb28181Config {
        let addr = SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 5060));
        Gb28181ConfigBuilder::new(
            cheetah_signal_types::TenantId::generate(),
            "3402000000",
            addr,
        )
        .build()
    }

    #[test]
    fn parse_keepalive() -> Result<(), crate::Gb28181ModuleError> {
        let xml = b"<?xml version=\"1.0\"?><Notify><CmdType>Keepalive</CmdType><SN>1</SN><DeviceID>34020000001320000001</DeviceID><Status>OK</Status></Notify>";
        let envelope = parse_xml(xml, &test_config())?;
        let msg = envelope.into_message();
        assert_eq!(msg.cmd_type, "Keepalive");
        assert_eq!(msg.device_id.as_deref(), Some("34020000001320000001"));
        assert_eq!(msg.status.as_deref(), Some("OK"));
        Ok(())
    }

    #[test]
    fn parse_catalog_response() -> Result<(), crate::Gb28181ModuleError> {
        let xml = br#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <SumNum>1</SumNum>
  <ItemList>
    <Item>
      <DeviceID>34020000001320000001</DeviceID>
      <Name>Camera</Name>
      <Status>ON</Status>
      <Parental>0</Parental>
    </Item>
  </ItemList>
</Response>"#;
        let envelope = parse_xml(xml, &test_config())?;
        let msg = envelope.into_message();
        assert_eq!(msg.cmd_type, "Catalog");
        let list = msg
            .item_list
            .as_ref()
            .ok_or_else(|| crate::Gb28181ModuleError::Xml("missing ItemList".into()))?;
        assert_eq!(list.item.len(), 1);
        assert_eq!(
            list.item[0].device_id.as_deref(),
            Some("34020000001320000001")
        );
        Ok(())
    }

    #[test]
    fn rejects_doctype() {
        let xml = b"<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY e \"1\">]><Notify><CmdType>Keepalive</CmdType></Notify>";
        assert!(parse_xml(xml, &test_config()).is_err());
    }

    #[test]
    fn rejects_oversized() {
        let mut config = test_config();
        config.xml_limits.max_body_bytes = 10;
        let xml = b"<?xml version=\"1.0\"?><Notify><CmdType>Keepalive</CmdType></Notify>";
        assert!(parse_xml(xml, &config).is_err());
    }

    #[test]
    fn rejects_deep_nesting() {
        let mut config = test_config();
        config.xml_limits.max_depth = 3;
        let xml = b"<?xml version=\"1.0\"?><a><b><c><d>deep</d></c></b></a>";
        assert!(parse_xml(xml, &config).is_err());
    }

    #[test]
    fn self_closing_elements_do_not_inflate_depth() {
        let mut config = test_config();
        config.xml_limits.max_depth = 3;
        let xml = br#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>1</SN>
  <DeviceID>34020000001320000001</DeviceID>
  <SumNum>1</SumNum>
  <ItemList>
    <Item>
      <Info/>
    </Item>
  </ItemList>
</Response>"#;
        let result = parse_xml(xml, &config);
        assert!(result.is_ok(), "{:#?}", result.err());
    }

    #[test]
    fn charset_policy_rejects_gbk_when_utf8_strict() {
        let mut config = test_config();
        config.charset_policy = crate::config::CharsetPolicy::Utf8;
        let xml = b"<?xml version=\"1.0\" encoding=\"GBK\"?><Notify><CmdType>Keepalive</CmdType><DeviceID>34020000001320000001</DeviceID><Status>OK</Status></Notify>";
        assert!(parse_xml(xml, &config).is_err());
    }

    #[test]
    fn charset_policy_allows_gbk_when_enabled() {
        let mut config = test_config();
        config.charset_policy = crate::config::CharsetPolicy::Gbk;
        let xml = b"<?xml version=\"1.0\" encoding=\"GBK\"?><Notify><CmdType>Keepalive</CmdType><DeviceID>34020000001320000001</DeviceID><Status>OK</Status></Notify>";
        assert!(parse_xml(xml, &config).is_ok());
    }

    #[test]
    fn encode_catalog_query() -> Result<(), crate::Gb28181ModuleError> {
        let body = encode_request("Query", "Catalog", 1, "34020000001320000001", &[])?;
        assert!(body.contains("<CmdType>Catalog</CmdType>"));
        assert!(body.contains("<SN>1</SN>"));
        Ok(())
    }
}

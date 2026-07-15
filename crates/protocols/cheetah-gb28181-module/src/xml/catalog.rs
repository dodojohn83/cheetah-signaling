//! GB28181 Catalog response parsing.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use crate::error::AccessError;

/// Parsed content of a GB28181 `Catalog` response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogResponse {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// `SumNum` attribute declaring total item count across all fragments.
    pub sum_num: u32,
    /// `Num` attribute declaring the number of items in this fragment.
    pub num: u32,
    /// Items in this fragment.
    pub items: Vec<CatalogItem>,
}

/// A single channel/item in a GB28181 catalog.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogItem {
    /// Channel/device identifier.
    pub device_id: String,
    /// Display name.
    pub name: Option<String>,
    /// Manufacturer.
    pub manufacturer: Option<String>,
    /// Model.
    pub model: Option<String>,
    /// Owner.
    pub owner: Option<String>,
    /// Civil code.
    pub civil_code: Option<String>,
    /// Block.
    pub block: Option<String>,
    /// Address.
    pub address: Option<String>,
    /// Parental flag.
    pub parental: Option<String>,
    /// Parent ID.
    pub parent_id: Option<String>,
    /// Safety way.
    pub safety_way: Option<String>,
    /// Register way.
    pub register_way: Option<String>,
    /// Certification number.
    pub cert_num: Option<String>,
    /// Certifiable flag.
    pub certifiable: Option<String>,
    /// Error code.
    pub err_code: Option<String>,
    /// End time.
    pub end_time: Option<String>,
    /// Secrecy flag.
    pub secrecy: Option<String>,
    /// IP address.
    pub ip_address: Option<String>,
    /// Port.
    pub port: Option<String>,
    /// Status (ON/OFF).
    pub status: Option<String>,
    /// Longitude.
    pub longitude: Option<String>,
    /// Latitude.
    pub latitude: Option<String>,
}

/// Parses a `Catalog` response body.
pub fn parse_catalog(body: &[u8]) -> Result<CatalogResponse, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_catalog(&root)
}

fn extract_catalog(root: &XmlElement) -> Result<CatalogResponse, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "Catalog" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    let device_list = root
        .child("DeviceList")
        .ok_or_else(|| AccessError::InvalidXml("missing DeviceList".to_string()))?;

    let num = device_list
        .attributes
        .get("Num")
        .map(|v| parse_u32(v))
        .unwrap_or(0);

    Ok(CatalogResponse {
        sn: root.child_text("SN").unwrap_or_default(),
        device_id: root.child_text("DeviceID").unwrap_or_default(),
        sum_num: parse_u32(&root.child_text("SumNum").unwrap_or_default()),
        num,
        items: device_list
            .children
            .iter()
            .filter(|c| c.name == "Item")
            .map(parse_item)
            .collect(),
    })
}

fn parse_item(item: &XmlElement) -> CatalogItem {
    CatalogItem {
        device_id: item.child_text("DeviceID").unwrap_or_default(),
        name: item.child_text("Name"),
        manufacturer: item.child_text("Manufacturer"),
        model: item.child_text("Model"),
        owner: item.child_text("Owner"),
        civil_code: item.child_text("CivilCode"),
        block: item.child_text("Block"),
        address: item.child_text("Address"),
        parental: item.child_text("Parental"),
        parent_id: item.child_text("ParentID"),
        safety_way: item.child_text("SafetyWay"),
        register_way: item.child_text("RegisterWay"),
        cert_num: item.child_text("CertNum"),
        certifiable: item.child_text("Certifiable"),
        err_code: item.child_text("ErrCode"),
        end_time: item.child_text("EndTime"),
        secrecy: item.child_text("Secrecy"),
        ip_address: item.child_text("IPAddress"),
        port: item.child_text("Port"),
        status: item.child_text("Status"),
        longitude: item.child_text("Longitude"),
        latitude: item.child_text("Latitude"),
    }
}

fn parse_u32(value: &str) -> u32 {
    value.trim().parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_valid_catalog() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>Catalog</CmdType>
    <SN>2</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <SumNum>1</SumNum>
    <DeviceList Num="1">
        <Item>
            <DeviceID>34020000001320000001</DeviceID>
            <Name>Camera 1</Name>
            <Status>ON</Status>
        </Item>
    </DeviceList>
</Response>"#;
        let catalog = parse_catalog(body).unwrap();
        assert_eq!(catalog.sn, "2");
        assert_eq!(catalog.device_id, "34020000001320000001");
        assert_eq!(catalog.sum_num, 1);
        assert_eq!(catalog.num, 1);
        assert_eq!(catalog.items.len(), 1);
        assert_eq!(catalog.items[0].device_id, "34020000001320000001");
        assert_eq!(catalog.items[0].name.as_deref(), Some("Camera 1"));
        assert_eq!(catalog.items[0].status.as_deref(), Some("ON"));
    }
}

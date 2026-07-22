//! GB28181 Catalog response/query parsing and encoding.

use std::collections::HashMap;

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use super::writer::encode_xml;
use crate::error::AccessError;
use cheetah_domain::{CompatibilityCapability, CompatibilityProfile};

/// Parsed content of a GB28181 `Catalog` response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogResponse {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// `SumNum` attribute declaring total item count across all fragments.
    pub sum_num: u32,
    /// Declared number of items in this fragment from the `Num` attribute. This
    /// may be greater than the length of `items` when some `Item` elements had
    /// missing or empty `DeviceID` values and were dropped. When the `Num`
    /// attribute is missing or invalid this falls back to the well-formed count.
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
    extract_catalog_with_profile(&root, &CompatibilityProfile::default())
}

pub(crate) fn extract_catalog_with_profile(
    root: &XmlElement,
    profile: &CompatibilityProfile,
) -> Result<CatalogResponse, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "Catalog" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    let allow_notify = profile.has(CompatibilityCapability::CatalogNotify);
    if root.name != "Response" && !(allow_notify && root.name == "Notify") {
        return Err(AccessError::InvalidXml(format!(
            "unexpected Catalog root element: {}",
            root.name
        )));
    }

    let device_list = root
        .child("DeviceList")
        .ok_or_else(|| AccessError::InvalidXml("missing DeviceList".to_string()))?;

    let sn = root.require_child_text("SN")?;
    let device_id = root.require_child_text("DeviceID")?;
    let sum_num = parse_u32(&root.require_child_text("SumNum")?)?;

    let mut dropped = 0u32;
    let items: Vec<CatalogItem> = device_list
        .children
        .iter()
        .filter(|c| c.name == "Item")
        .filter_map(|item| {
            let parsed = parse_item(item);
            if parsed.is_none() {
                dropped += 1;
            }
            parsed
        })
        .collect();

    if dropped > 0 {
        tracing::warn!(
            sn = %sn,
            device_id = %device_id,
            dropped,
            "catalog item(s) dropped due to missing or empty DeviceID"
        );
    }

    // Use the declared `Num` attribute when present and valid so that
    // downstream fragment assembly can detect the final fragment even when some
    // `Item` elements are malformed and dropped. Fall back to the number of
    // well-formed items we actually parsed when the attribute is missing or
    // invalid.
    let (declared_num, declared_num_from_wire) = device_list
        .attributes
        .get("Num")
        .and_then(|v| parse_u32(v).ok())
        .map(|n| (n, true))
        .unwrap_or((items.len() as u32, false));

    if declared_num_from_wire {
        // Count all <Item> elements present on the wire, including the malformed
        // ones intentionally dropped, so a single bad entry does not reject the
        // whole catalog. GB28181 devices frequently declare inaccurate counts or
        // split a directory across multiple fragments, so mismatches are accepted
        // by default and logged as a diagnostic; callers that need strict
        // validation can post-process `num`/`sum_num` against `items.len()`.
        let present = items.len() as u32 + dropped;
        if declared_num != present {
            tracing::warn!(
                sn = %sn,
                device_id = %device_id,
                declared_num,
                present,
                "catalog Num does not match parsed item count; accepting anyway"
            );
        }
        if sum_num > 0 && declared_num > sum_num {
            tracing::warn!(
                sn = %sn,
                device_id = %device_id,
                declared_num,
                sum_num,
                "catalog Num exceeds SumNum; accepting anyway"
            );
        }
    }

    Ok(CatalogResponse {
        sn,
        device_id,
        sum_num,
        num: declared_num,
        items,
    })
}

fn parse_item(item: &XmlElement) -> Option<CatalogItem> {
    let device_id = item.child_text("DeviceID")?;
    if device_id.is_empty() {
        return None;
    }
    Some(CatalogItem {
        device_id,
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
    })
}

/// Parsed content of a GB28181 `Catalog` query from an upstream platform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogQuery {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
}

/// Parses a `Catalog` query body.
pub fn parse_catalog_query(body: &[u8]) -> Result<CatalogQuery, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    if root.name != "Query" {
        return Err(AccessError::InvalidXml(format!(
            "expected Query root, got {}",
            root.name
        )));
    }
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "Catalog" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }
    let device_id = root.require_child_text("DeviceID")?;
    Ok(CatalogQuery {
        sn: root.require_child_text("SN")?,
        device_id,
    })
}

fn child_element(name: &str, text: &str) -> XmlElement {
    XmlElement {
        name: name.to_string(),
        attributes: HashMap::new(),
        text: text.to_string(),
        children: Vec::new(),
    }
}

fn add_optional_child(parent: &mut XmlElement, name: &str, value: &Option<String>) {
    if let Some(value) = value {
        parent.children.push(child_element(name, value));
    }
}

/// Builds a `Catalog` response XML payload for one fragment.
pub fn build_catalog_response(
    sn: &str,
    device_id: &str,
    sum_num: u32,
    items: &[CatalogItem],
) -> Result<String, AccessError> {
    let mut root = child_element("Response", "");
    root.children.push(child_element("CmdType", "Catalog"));
    root.children.push(child_element("SN", sn));
    root.children.push(child_element("DeviceID", device_id));
    root.children
        .push(child_element("SumNum", &sum_num.to_string()));

    let mut device_list = child_element("DeviceList", "");
    device_list
        .attributes
        .insert("Num".to_string(), items.len().to_string());
    for item in items {
        let mut item_el = child_element("Item", "");
        item_el
            .children
            .push(child_element("DeviceID", &item.device_id));
        add_optional_child(&mut item_el, "Name", &item.name);
        add_optional_child(&mut item_el, "Manufacturer", &item.manufacturer);
        add_optional_child(&mut item_el, "Model", &item.model);
        add_optional_child(&mut item_el, "Owner", &item.owner);
        add_optional_child(&mut item_el, "CivilCode", &item.civil_code);
        add_optional_child(&mut item_el, "Block", &item.block);
        add_optional_child(&mut item_el, "Address", &item.address);
        add_optional_child(&mut item_el, "Parental", &item.parental);
        add_optional_child(&mut item_el, "ParentID", &item.parent_id);
        add_optional_child(&mut item_el, "SafetyWay", &item.safety_way);
        add_optional_child(&mut item_el, "RegisterWay", &item.register_way);
        add_optional_child(&mut item_el, "CertNum", &item.cert_num);
        add_optional_child(&mut item_el, "Certifiable", &item.certifiable);
        add_optional_child(&mut item_el, "ErrCode", &item.err_code);
        add_optional_child(&mut item_el, "EndTime", &item.end_time);
        add_optional_child(&mut item_el, "Secrecy", &item.secrecy);
        add_optional_child(&mut item_el, "IPAddress", &item.ip_address);
        add_optional_child(&mut item_el, "Port", &item.port);
        add_optional_child(&mut item_el, "Status", &item.status);
        add_optional_child(&mut item_el, "Longitude", &item.longitude);
        add_optional_child(&mut item_el, "Latitude", &item.latitude);
        device_list.children.push(item_el);
    }
    root.children.push(device_list);
    encode_xml(&root, true)
}

fn parse_u32(value: &str) -> Result<u32, AccessError> {
    value
        .trim()
        .parse()
        .map_err(|_| AccessError::InvalidXml(format!("invalid numeric value: {value}")))
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

    #[test]
    fn parse_valid_catalog_query() {
        let body = br#"<?xml version="1.0"?>
<Query>
    <CmdType>Catalog</CmdType>
    <SN>7</SN>
    <DeviceID>34020000001320000001</DeviceID>
</Query>"#;
        let query = parse_catalog_query(body).unwrap();
        assert_eq!(query.sn, "7");
        assert_eq!(query.device_id, "34020000001320000001");
    }

    #[test]
    fn parse_catalog_query_rejects_non_catalog() {
        let body = br#"<?xml version="1.0"?>
<Query>
    <CmdType>DeviceInfo</CmdType>
    <SN>1</SN>
    <DeviceID>34020000001320000001</DeviceID>
</Query>"#;
        assert!(parse_catalog_query(body).is_err());
    }

    #[test]
    fn build_catalog_response_round_trips() {
        let items = vec![CatalogItem {
            device_id: "34020000001320000001".to_string(),
            name: Some("Camera 1".to_string()),
            status: Some("ON".to_string()),
            ..Default::default()
        }];
        let xml = build_catalog_response("3", "34020000001320000001", 1, &items).unwrap();
        let parsed = parse_catalog(xml.as_bytes()).unwrap();
        assert_eq!(parsed.sn, "3");
        assert_eq!(parsed.device_id, "34020000001320000001");
        assert_eq!(parsed.sum_num, 1);
        assert_eq!(parsed.num, 1);
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].device_id, "34020000001320000001");
        assert_eq!(parsed.items[0].name.as_deref(), Some("Camera 1"));
        assert_eq!(parsed.items[0].status.as_deref(), Some("ON"));
    }

    #[test]
    fn build_empty_catalog_response_has_zero_sum() {
        let xml = build_catalog_response("1", "34020000001320000001", 0, &[]).unwrap();
        let parsed = parse_catalog(xml.as_bytes()).unwrap();
        assert_eq!(parsed.sum_num, 0);
        assert_eq!(parsed.num, 0);
        assert!(parsed.items.is_empty());
    }

    #[test]
    fn malformed_item_does_not_reject_whole_catalog() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>Catalog</CmdType>
    <SN>2</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <SumNum>2</SumNum>
    <DeviceList Num="2">
        <Item>
            <DeviceID>34020000001320000001</DeviceID>
            <Name>Camera 1</Name>
            <Status>ON</Status>
        </Item>
        <Item>
            <Name>Camera 2</Name>
            <Status>OFF</Status>
        </Item>
    </DeviceList>
</Response>"#;
        let catalog = parse_catalog(body).unwrap();
        assert_eq!(catalog.num, 2);
        assert_eq!(catalog.sum_num, 2);
        assert_eq!(catalog.items.len(), 1);
        assert_eq!(catalog.items[0].device_id, "34020000001320000001");
    }

    #[test]
    fn inconsistent_counts_are_accepted_by_default() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>Catalog</CmdType>
    <SN>3</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <SumNum>3</SumNum>
    <DeviceList Num="5">
        <Item><DeviceID>34020000001320000001</DeviceID><Name>Cam1</Name><Status>ON</Status></Item>
        <Item><DeviceID>34020000001320000002</DeviceID><Name>Cam2</Name><Status>ON</Status></Item>
        <Item><DeviceID>34020000001320000003</DeviceID><Name>Cam3</Name><Status>ON</Status></Item>
    </DeviceList>
</Response>"#;
        let catalog = parse_catalog(body).unwrap();
        assert_eq!(catalog.items.len(), 3);
        assert_eq!(catalog.num, 5);
        assert_eq!(catalog.sum_num, 3);
    }
}

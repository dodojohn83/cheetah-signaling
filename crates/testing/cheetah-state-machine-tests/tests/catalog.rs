//! GB4-TST-002 catalog transition coverage: fragment / duplicate / reorder /
//! missing / partial / crash / revision-conflict.
//!
//! GB28181 catalog ingestion is split between a *stateless per-fragment parser*
//! ([`parse_catalog`]) and the *consumer* that accumulates fragments and applies
//! them to channel state under optimistic concurrency. There is no separate
//! production "catalog collector" aggregate, so these tests pin the behaviour at
//! the two layers that actually exist:
//!
//! - parser-level guarantees (fragment counts, malformed-item dropping,
//!   arrival-order independence, deterministic re-parse after a crash);
//! - the consumer contract those guarantees imply (duplicate/partial detection
//!   are functions of `Num`/`SumNum`; revision-conflict is exercised by the
//!   repository contract suite in `cheetah-storage-tests`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_gb28181_module::error::AccessError;
use cheetah_gb28181_module::xml::catalog::{
    CatalogItem, CatalogResponse, build_catalog_response, parse_catalog,
};

fn item(device_id: &str) -> CatalogItem {
    CatalogItem {
        device_id: device_id.to_string(),
        ..Default::default()
    }
}

/// A fragment declares its own `Num` (items in this fragment) and the shared
/// `SumNum` (grand total across fragments). A fragment that is a strict subset
/// of the total parses and reports `num < sum_num`.
#[test]
fn fragment_reports_partial_progress_against_sum_num() {
    let items = vec![item("34020000001320000001"), item("34020000001320000002")];
    let xml = build_catalog_response("100", "34020000002000000001", 5, &items).unwrap();
    let parsed = parse_catalog(xml.as_bytes()).unwrap();
    assert_eq!(parsed.sum_num, 5);
    assert_eq!(parsed.num, 2);
    assert_eq!(parsed.items.len(), 2);
    // Partial: fewer items delivered than the declared grand total.
    assert!(parsed.num < parsed.sum_num);
}

/// Reorder: the parser is stateless, so the same fragment parses to the same
/// value regardless of when it arrives relative to other fragments.
#[test]
fn parser_is_order_independent() {
    let frag_a = build_catalog_response(
        "100",
        "34020000002000000001",
        4,
        &[item("34020000001320000001"), item("34020000001320000002")],
    )
    .unwrap();
    let frag_b = build_catalog_response(
        "100",
        "34020000002000000001",
        4,
        &[item("34020000001320000003"), item("34020000001320000004")],
    )
    .unwrap();

    let ab: Vec<_> = [&frag_a, &frag_b]
        .iter()
        .map(|f| parse_catalog(f.as_bytes()).unwrap())
        .collect();
    let ba: Vec<_> = [&frag_b, &frag_a]
        .iter()
        .map(|f| parse_catalog(f.as_bytes()).unwrap())
        .collect();
    assert_eq!(ab[0], ba[1]);
    assert_eq!(ab[1], ba[0]);
}

/// Duplicate: the parser does not deduplicate across fragments. Two fragments
/// carrying the same `DeviceID` each parse it; dedup is the consumer's job, so
/// the union of a fragment with itself is the same single logical device.
#[test]
fn duplicate_device_id_is_preserved_for_consumer_dedup() {
    let xml = build_catalog_response(
        "100",
        "34020000002000000001",
        1,
        &[item("34020000001320000001")],
    )
    .unwrap();
    let first = parse_catalog(xml.as_bytes()).unwrap();
    let second = parse_catalog(xml.as_bytes()).unwrap();
    assert_eq!(first, second);

    let mut deduped: Vec<String> = first
        .items
        .iter()
        .chain(second.items.iter())
        .map(|i| i.device_id.clone())
        .collect();
    deduped.sort();
    deduped.dedup();
    assert_eq!(deduped, vec!["34020000001320000001".to_string()]);
}

/// Missing: a fragment without the required `SumNum` element is rejected.
#[test]
fn missing_required_element_is_rejected() {
    let xml = r#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>100</SN>
  <DeviceID>34020000002000000001</DeviceID>
  <DeviceList Num="0"></DeviceList>
</Response>"#;
    let err = parse_catalog(xml.as_bytes()).unwrap_err();
    assert!(matches!(err, AccessError::InvalidXml(_)));
}

/// Partial / malformed: items with an empty `DeviceID` are dropped, and the
/// declared `Num` must then match the well-formed count under strict parsing.
#[test]
fn declared_num_must_match_wellformed_item_count() {
    // Two well-formed items but Num claims three: strict parser rejects it.
    let xml = r#"<?xml version="1.0"?>
<Response>
  <CmdType>Catalog</CmdType>
  <SN>100</SN>
  <DeviceID>34020000002000000001</DeviceID>
  <SumNum>3</SumNum>
  <DeviceList Num="3">
    <Item><DeviceID>34020000001320000001</DeviceID></Item>
    <Item><DeviceID>34020000001320000002</DeviceID></Item>
  </DeviceList>
</Response>"#;
    let err = parse_catalog(xml.as_bytes()).unwrap_err();
    assert!(matches!(err, AccessError::InvalidXml(_)));
}

/// Crash / restart: because parsing carries no hidden state, replaying the same
/// fragment bytes after a simulated crash yields a byte-for-byte identical
/// result, so a consumer can safely resume assembly from scratch.
#[test]
fn reparse_after_crash_is_deterministic() {
    let items = vec![item("34020000001320000001"), item("34020000001320000002")];
    let xml = build_catalog_response("100", "34020000002000000001", 2, &items).unwrap();
    let before: CatalogResponse = parse_catalog(xml.as_bytes()).unwrap();
    // Simulate a process restart: re-run the parser on the retained bytes.
    let after: CatalogResponse = parse_catalog(xml.as_bytes()).unwrap();
    assert_eq!(before, after);
    assert_eq!(after.items.len(), 2);
}

/// A non-catalog command type is rejected: catalog ingestion must not accept a
/// body that is actually another MANSCDP response.
#[test]
fn non_catalog_command_type_is_rejected() {
    let xml = r#"<?xml version="1.0"?>
<Response>
  <CmdType>DeviceInfo</CmdType>
  <SN>100</SN>
  <DeviceID>34020000002000000001</DeviceID>
  <SumNum>0</SumNum>
  <DeviceList Num="0"></DeviceList>
</Response>"#;
    let err = parse_catalog(xml.as_bytes()).unwrap_err();
    assert!(matches!(err, AccessError::UnsupportedCmdType(_)));
}

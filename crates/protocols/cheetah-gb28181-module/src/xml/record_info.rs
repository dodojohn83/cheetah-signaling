//! GB28181 RecordInfo response parsing.

use super::element::XmlElement;
use super::limits::XmlLimits;
use super::reader::parse_xml;
use crate::error::AccessError;

/// Parsed content of a GB28181 `RecordInfo` response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordInfoResponse {
    /// Sequence number from the `<SN>` element.
    pub sn: String,
    /// Device identifier from the `<DeviceID>` element.
    pub device_id: String,
    /// Device name.
    pub name: Option<String>,
    /// `SumNum` declaring total record count across all fragments.
    pub sum_num: u32,
    /// Number of well-formed records actually parsed in this fragment. This may
    /// be less than the `Num` attribute if some `Item` elements had missing or
    /// empty `DeviceID` values and were dropped.
    pub num: u32,
    /// Records in this fragment.
    pub items: Vec<RecordItem>,
}

/// A single record item from a GB28181 `RecordInfo` response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordItem {
    /// Channel/device identifier.
    pub device_id: String,
    /// Channel name.
    pub name: Option<String>,
    /// File path or stream address.
    pub file_path: Option<String>,
    /// Start time.
    pub start_time: Option<String>,
    /// End time.
    pub end_time: Option<String>,
    /// Secrecy flag.
    pub secrecy: Option<String>,
    /// Record type.
    pub record_type: Option<String>,
    /// Recorder ID.
    pub recorder_id: Option<String>,
    /// File size.
    pub file_size: Option<String>,
}

/// Parses a `RecordInfo` response body.
pub fn parse_record_info(body: &[u8]) -> Result<RecordInfoResponse, AccessError> {
    let root = parse_xml(body, &XmlLimits::default())?;
    extract_record_info(&root)
}

pub(crate) fn extract_record_info(root: &XmlElement) -> Result<RecordInfoResponse, AccessError> {
    let cmd_type = root
        .child_text("CmdType")
        .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
    if cmd_type != "RecordInfo" {
        return Err(AccessError::UnsupportedCmdType(cmd_type));
    }

    let record_list = root
        .child("RecordList")
        .ok_or_else(|| AccessError::InvalidXml("missing RecordList".to_string()))?;

    let sn = root.require_child_text("SN")?;
    let device_id = root.require_child_text("DeviceID")?;
    let name = root.child_text("Name");
    let sum_num = parse_u32(&root.require_child_text("SumNum")?)?;

    let mut dropped = 0u32;
    let items: Vec<RecordItem> = record_list
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
            "record info item(s) dropped due to missing or empty DeviceID"
        );
    }

    // Ignore a missing or malformed `Num` attribute and use the number of
    // well-formed items actually parsed.
    let num = items.len() as u32;

    Ok(RecordInfoResponse {
        sn,
        device_id,
        name,
        sum_num,
        num,
        items,
    })
}

fn parse_item(item: &XmlElement) -> Option<RecordItem> {
    let device_id = item.child_text("DeviceID")?;
    if device_id.is_empty() {
        return None;
    }
    Some(RecordItem {
        device_id,
        name: item.child_text("Name"),
        file_path: item.child_text("FilePath"),
        start_time: item.child_text("StartTime"),
        end_time: item.child_text("EndTime"),
        secrecy: item.child_text("Secrecy"),
        record_type: item.child_text("Type"),
        recorder_id: item.child_text("RecorderID"),
        file_size: item.child_text("FileSize"),
    })
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
    fn parse_valid_record_info() {
        let body = br#"<?xml version="1.0"?>
<Response>
    <CmdType>RecordInfo</CmdType>
    <SN>7</SN>
    <DeviceID>34020000001320000001</DeviceID>
    <Name>Camera 1</Name>
    <SumNum>1</SumNum>
    <RecordList Num="1">
        <Item>
            <DeviceID>34020000001320000001</DeviceID>
            <Name>Channel 1</Name>
            <FilePath>/path/to/file</FilePath>
            <StartTime>2026-07-13T00:00:00</StartTime>
            <EndTime>2026-07-13T01:00:00</EndTime>
            <Secrecy>0</Secrecy>
            <Type>time</Type>
            <FileSize>1024</FileSize>
        </Item>
    </RecordList>
</Response>"#;
        let info = parse_record_info(body).unwrap();
        assert_eq!(info.sn, "7");
        assert_eq!(info.device_id, "34020000001320000001");
        assert_eq!(info.name.as_deref(), Some("Camera 1"));
        assert_eq!(info.sum_num, 1);
        assert_eq!(info.num, 1);
        assert_eq!(info.items.len(), 1);
        assert_eq!(info.items[0].device_id, "34020000001320000001");
        assert_eq!(info.items[0].file_path.as_deref(), Some("/path/to/file"));
        assert_eq!(info.items[0].file_size.as_deref(), Some("1024"));
    }
}

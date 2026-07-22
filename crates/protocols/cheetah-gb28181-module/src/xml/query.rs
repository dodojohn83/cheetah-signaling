//! GB28181 query request builders.

use super::element::XmlElement;
use super::writer::encode_xml;
use crate::error::AccessError;
use cheetah_domain::{QueryCommand, QueryKind};
use cheetah_signal_types::UtcTimestamp;

fn text_child(name: &str, text: &str) -> XmlElement {
    XmlElement {
        name: name.to_string(),
        text: text.to_string(),
        ..Default::default()
    }
}

fn gb_time(ts: UtcTimestamp) -> Result<String, AccessError> {
    let s = ts
        .to_rfc3339()
        .map_err(|e| AccessError::Internal(e.to_string()))?;
    // GB28181 wire format is YYYY-MM-DDTHH:MM:SS without timezone suffix.
    Ok(s.get(..19).unwrap_or(&s).to_string())
}

fn cmd_type(kind: QueryKind) -> &'static str {
    match kind {
        QueryKind::Catalog => "Catalog",
        QueryKind::DeviceInfo => "DeviceInfo",
        QueryKind::DeviceStatus => "DeviceStatus",
        QueryKind::RecordInfo => "RecordInfo",
        QueryKind::PresetQuery => "PresetQuery",
        QueryKind::ConfigDownload => "ConfigDownload",
    }
}

/// A GB28181 `<Query>` request.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryRequest {
    /// Sequence number.
    pub sn: String,
    /// Device or channel identifier.
    pub device_id: String,
    /// Query kind.
    pub kind: QueryKind,
    /// Optional start of a record window.
    pub start_time: Option<UtcTimestamp>,
    /// Optional end of a record window.
    pub end_time: Option<UtcTimestamp>,
    /// Optional configuration type for `ConfigDownload`.
    pub config_type: Option<String>,
    /// Optional playback speed scale for `RecordInfo`.
    pub scale: Option<f64>,
}

impl QueryRequest {
    /// Creates a query request from a domain command.
    pub fn from_command(
        sn: impl Into<String>,
        device_id: impl Into<String>,
        command: &QueryCommand,
    ) -> Self {
        Self {
            sn: sn.into(),
            device_id: device_id.into(),
            kind: command.kind,
            start_time: command.start_time,
            end_time: command.end_time,
            config_type: command.config_type.clone(),
            scale: command.scale,
        }
    }

    /// Encodes the request as a `<Query>` XML body.
    pub fn encode_xml(&self) -> Result<String, AccessError> {
        let mut children = vec![
            text_child("CmdType", cmd_type(self.kind)),
            text_child("SN", &self.sn),
            text_child("DeviceID", &self.device_id),
        ];

        match self.kind {
            QueryKind::RecordInfo => {
                let start = self.start_time.ok_or_else(|| {
                    AccessError::InvalidXml("RecordInfo requires start_time".to_string())
                })?;
                let end = self.end_time.ok_or_else(|| {
                    AccessError::InvalidXml("RecordInfo requires end_time".to_string())
                })?;
                children.push(text_child("StartTime", &gb_time(start)?));
                children.push(text_child("EndTime", &gb_time(end)?));
                children.push(text_child("Type", "all"));
                if let Some(scale) = self.scale {
                    children.push(text_child("Scale", &scale.to_string()));
                }
            }
            QueryKind::ConfigDownload => {
                let config_type = self.config_type.as_deref().unwrap_or("BasicParam");
                children.push(text_child("ConfigType", config_type));
            }
            _ => {}
        }

        let root = XmlElement {
            name: "Query".to_string(),
            children,
            ..Default::default()
        };
        encode_xml(&root, true)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use cheetah_domain::QueryCommand;
    use cheetah_signal_types::UtcTimestamp;

    #[test]
    fn catalog_query_xml() {
        let req = QueryRequest {
            sn: "7".to_string(),
            device_id: "34020000001320000001".to_string(),
            kind: QueryKind::Catalog,
            start_time: None,
            end_time: None,
            config_type: None,
            scale: None,
        };
        let xml = req.encode_xml().unwrap();
        assert!(xml.contains("<CmdType>Catalog</CmdType>"));
        assert!(xml.contains("<SN>7</SN>"));
        assert!(xml.contains("<DeviceID>34020000001320000001</DeviceID>"));
    }

    #[test]
    fn record_info_query_xml() {
        let ts = UtcTimestamp::parse_rfc3339("2026-03-02T00:00:00Z").unwrap();
        let req = QueryRequest {
            sn: "32".to_string(),
            device_id: "34020000001320000105".to_string(),
            kind: QueryKind::RecordInfo,
            start_time: Some(ts),
            end_time: Some(ts),
            config_type: None,
            scale: None,
        };
        let xml = req.encode_xml().unwrap();
        assert!(xml.contains("<CmdType>RecordInfo</CmdType>"));
        assert!(xml.contains("<StartTime>2026-03-02T00:00:00</StartTime>"));
        assert!(xml.contains("<Type>all</Type>"));
    }

    #[test]
    fn config_download_query_xml() {
        let req = QueryRequest {
            sn: "42".to_string(),
            device_id: "34020000001320000264".to_string(),
            kind: QueryKind::ConfigDownload,
            start_time: None,
            end_time: None,
            config_type: Some("BasicParam".to_string()),
            scale: None,
        };
        let xml = req.encode_xml().unwrap();
        assert!(xml.contains("<CmdType>ConfigDownload</CmdType>"));
        assert!(xml.contains("<ConfigType>BasicParam</ConfigType>"));
    }

    #[test]
    fn from_command_copies_kind_and_times() {
        let ts = UtcTimestamp::parse_rfc3339("2026-03-02T10:00:00Z").unwrap();
        let command = QueryCommand {
            kind: QueryKind::RecordInfo,
            channel_id: None,
            start_time: Some(ts),
            end_time: Some(ts),
            config_type: Some("BasicParam".to_string()),
            scale: Some(2.0),
        };
        let req = QueryRequest::from_command("9", "34020000001320000001", &command);
        assert_eq!(req.sn, "9");
        assert_eq!(req.device_id, "34020000001320000001");
        assert_eq!(req.kind, QueryKind::RecordInfo);
        assert_eq!(req.start_time, Some(ts));
        assert_eq!(req.scale, Some(2.0));
    }

    #[test]
    fn preset_query_xml() {
        let req = QueryRequest {
            sn: "11".to_string(),
            device_id: "34020000001320000001".to_string(),
            kind: QueryKind::PresetQuery,
            start_time: None,
            end_time: None,
            config_type: None,
            scale: None,
        };
        let xml = req.encode_xml().unwrap();
        assert!(xml.contains("<CmdType>PresetQuery</CmdType>"));
    }
}

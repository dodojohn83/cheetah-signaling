//! Source readers for migration data.

use crate::error::MigrationError;
use crate::model::OldRecord;
use std::path::{Path, PathBuf};

/// Supported input formats.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceFormat {
    /// Comma-separated values.
    #[default]
    Csv,
    /// JSON array.
    Json,
}

impl std::str::FromStr for SourceFormat {
    type Err = MigrationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "csv" => Ok(Self::Csv),
            "json" => Ok(Self::Json),
            _ => Err(MigrationError::other(format!("unknown source format: {s}"))),
        }
    }
}

/// Reads `OldRecord` rows from a source file.
#[async_trait::async_trait]
pub trait RecordSource: Send + Sync {
    /// Reads all records from the source.
    async fn read_records(&self) -> Result<Vec<OldRecord>, MigrationError>;
}

/// File-based record source.
#[derive(Debug)]
pub struct FileSource {
    /// Path to the source file.
    path: PathBuf,
    /// Explicit format, or inferred from the extension.
    format: SourceFormat,
}

impl FileSource {
    /// Creates a new file source, inferring the format from the extension
    /// when not provided.
    pub fn new(
        path: impl AsRef<Path>,
        format: Option<SourceFormat>,
    ) -> Result<Self, MigrationError> {
        let path = path.as_ref().to_path_buf();
        let format = format.unwrap_or_else(|| Self::infer_format(&path));
        Ok(Self { path, format })
    }

    fn infer_format(path: &Path) -> SourceFormat {
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => SourceFormat::Json,
            _ => SourceFormat::Csv,
        }
    }
}

#[async_trait::async_trait]
impl RecordSource for FileSource {
    async fn read_records(&self) -> Result<Vec<OldRecord>, MigrationError> {
        let bytes = tokio::fs::read(&self.path)
            .await
            .map_err(|e| MigrationError::SourceRead {
                path: self.path.clone(),
                source: e,
            })?;

        match self.format {
            SourceFormat::Csv => read_csv(&bytes, &self.path),
            SourceFormat::Json => read_json(&bytes, &self.path),
        }
    }
}

fn read_csv(bytes: &[u8], path: &Path) -> Result<Vec<OldRecord>, MigrationError> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(bytes);
    let mut records = Vec::new();
    for result in reader.deserialize::<OldRecord>() {
        let record = result.map_err(|e| MigrationError::Csv {
            path: path.to_path_buf(),
            source: e,
        })?;
        if record.entity_type == crate::model::EntityType::Unknown
            && record.external_id.is_empty()
            && record.name.is_empty()
        {
            // Skip empty rows that sometimes appear at the end of CSV exports.
            continue;
        }
        records.push(record);
    }
    Ok(records)
}

fn read_json(bytes: &[u8], path: &Path) -> Result<Vec<OldRecord>, MigrationError> {
    serde_json::from_slice::<Vec<OldRecord>>(bytes).map_err(|e| MigrationError::Json {
        path: path.to_path_buf(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn rt() -> Result<tokio::runtime::Runtime, std::io::Error> {
        tokio::runtime::Runtime::new()
    }

    #[test]
    fn read_csv_parses_rows() -> Result<(), Box<dyn std::error::Error>> {
        let mut temp = tempfile::NamedTempFile::new()?;
        writeln!(temp, "entity_type,tenant_id,external_id,name,protocol,kind")?;
        writeln!(temp, "device,tenant-a,cam-01,Camera 1,gb28181,camera")?;
        temp.flush()?;

        let source = FileSource::new(temp.path(), Some(SourceFormat::Csv))?;
        let records = rt()?.block_on(source.read_records())?;

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].entity_type, crate::model::EntityType::Device);
        assert_eq!(records[0].external_id, "cam-01");
        assert_eq!(records[0].tenant_id, "tenant-a");
        Ok(())
    }

    #[test]
    fn read_json_parses_array() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"[
            {"entity_type": "channel", "tenant_id": "tenant-a", "external_id": "ch-1", "name": "CH1", "parent_device_id": "cam-01"}
        ]"#;
        let mut temp = tempfile::NamedTempFile::new()?;
        temp.write_all(json.as_bytes())?;
        temp.flush()?;

        let source = FileSource::new(temp.path(), Some(SourceFormat::Json))?;
        let records = rt()?.block_on(source.read_records())?;

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].entity_type, crate::model::EntityType::Channel);
        assert_eq!(records[0].external_id, "ch-1");
        Ok(())
    }
}

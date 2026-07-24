//! Integration tests for cheetah-signal-types.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_signal_types::{
    Deadline, DeviceId, DurationMs, MAX_PAGE_SIZE, MediaSessionId, MessageId, OwnerEpoch, Page,
    PageRequest, ProtocolIdentity, Revision, SignalConfig, SignalError, SignalErrorKind, TenantId,
    UtcTimestamp, config::LogFormat,
};
use std::str::FromStr;

#[test]
fn id_newtypes_are_distinct_and_serializable() -> Result<(), SignalError> {
    let tenant: TenantId = "550e8400-e29b-41d4-a716-446655440000".parse()?;
    let device: DeviceId = "550e8400-e29b-41d4-a716-446655440001".parse()?;
    let session: MediaSessionId = "550e8400-e29b-41d4-a716-446655440002".parse()?;

    assert_ne!(tenant.as_uuid(), device.as_uuid());
    assert_ne!(device.as_uuid(), session.as_uuid());

    let json = serde_json::to_string(&device).map_err(|e| {
        SignalError::new(SignalErrorKind::Internal, "json serialization failed").with_source(e)
    })?;
    assert!(json.contains("550e8400-e29b-41d4-a716-446655440001"));

    Ok(())
}

#[test]
fn invalid_id_strings_are_rejected() -> Result<(), SignalError> {
    let result = DeviceId::from_str("not-a-uuid");
    assert!(result.is_err());
    let err = result.err().ok_or_else(|| {
        SignalError::new(
            SignalErrorKind::Internal,
            "expected an error from invalid id",
        )
    })?;
    assert_eq!(err.kind(), SignalErrorKind::InvalidArgument);
    Ok(())
}

#[test]
fn protocol_identity_has_bounds() {
    assert!(ProtocolIdentity::new("device-123").is_ok());
    assert!(ProtocolIdentity::new("").is_err());
    let long = "x".repeat(300);
    assert!(ProtocolIdentity::new(&long).is_err());
}

#[test]
fn revision_and_epoch_default_to_zero() {
    let rev = Revision::default();
    let epoch = OwnerEpoch::default();
    assert_eq!(rev.0, 0);
    assert_eq!(epoch.0, 0);
}

#[test]
fn error_mapping_is_stable() {
    let err = SignalError::new(SignalErrorKind::NotFound, "device missing");
    assert_eq!(err.code(), "NOT_FOUND");
    assert!(!err.is_retryable());
    assert_eq!(err.to_http_status(), 404);
    assert_eq!(err.to_grpc_code(), 5);

    let retry = SignalError::new(SignalErrorKind::Unavailable, "down");
    assert!(retry.is_retryable());
}

#[test]
fn timestamp_and_duration_round_trip() -> Result<(), SignalError> {
    let now = UtcTimestamp::parse_rfc3339("2024-01-01T00:00:00Z")?;
    let later = now
        .checked_add(DurationMs::from_seconds(5))
        .ok_or_else(|| SignalError::new(SignalErrorKind::Internal, "overflow"))?;
    let remaining = later
        .checked_sub(DurationMs::from_seconds(5))
        .ok_or_else(|| SignalError::new(SignalErrorKind::Internal, "overflow"))?;
    assert_eq!(now.as_unix_seconds(), remaining.as_unix_seconds());

    let rfc = later.to_rfc3339()?;
    let parsed = UtcTimestamp::parse_rfc3339(&rfc)?;
    assert_eq!(later.as_unix_seconds(), parsed.as_unix_seconds());
    Ok(())
}

#[test]
fn deadline_elapsed_and_remaining_work() -> Result<(), SignalError> {
    let now = UtcTimestamp::parse_rfc3339("2024-01-01T00:00:00Z")?;
    let deadline = Deadline::from_now(now, DurationMs::from_seconds(10))
        .ok_or_else(|| SignalError::new(SignalErrorKind::Internal, "overflow"))?;

    assert!(!deadline.is_elapsed(now));
    let before = now
        .checked_add(DurationMs::from_seconds(5))
        .ok_or_else(|| SignalError::new(SignalErrorKind::Internal, "overflow"))?;
    let remaining = deadline
        .remaining(before)
        .ok_or_else(|| SignalError::new(SignalErrorKind::Internal, "no remaining"))?;
    assert!(remaining.as_millis() >= 4_000 && remaining.as_millis() <= 6_000);

    let after = now
        .checked_add(DurationMs::from_seconds(20))
        .ok_or_else(|| SignalError::new(SignalErrorKind::Internal, "overflow"))?;
    assert!(deadline.is_elapsed(after));
    assert!(deadline.remaining(after).is_none());

    Ok(())
}

#[test]
fn page_request_enforces_bounds() {
    assert!(PageRequest::new(0).is_err());
    assert!(PageRequest::new(1_001).is_err());
    assert!(PageRequest::new(20).is_ok());
}

#[test]
fn page_can_map_items() {
    let page = Page::new(vec![1, 2, 3])
        .with_next_cursor("cursor-1")
        .with_total(100);
    let mapped: Page<String> = page.map(|i| i.to_string());
    assert_eq!(mapped.items, ["1", "2", "3"]);
    assert_eq!(mapped.total, Some(100));
    assert_eq!(mapped.next_cursor.as_deref(), Some("cursor-1"));
}

#[test]
fn default_config_is_valid() -> Result<(), SignalError> {
    let config = SignalConfig::default();
    config.validate()?;
    let example = SignalConfig::example_toml()?;
    assert!(example.contains("http"));
    Ok(())
}

#[test]
fn session_reaper_knobs_are_bounded() {
    let mut config = SignalConfig::default();
    assert!(config.validate().is_ok());

    config.gb28181.session_reaper_max_per_tick = 0;
    assert!(config.validate().is_err());

    config.gb28181.session_reaper_max_per_tick =
        cheetah_signal_types::config::SESSION_REAPER_MAX_PER_TICK_LIMIT + 1;
    assert!(config.validate().is_err());

    config.gb28181.session_reaper_max_per_tick = 4_096;
    config.gb28181.session_reaper_batch_size = 0;
    assert!(config.validate().is_err());

    config.gb28181.session_reaper_batch_size = MAX_PAGE_SIZE + 1;
    assert!(config.validate().is_err());
}

#[test]
fn observability_diagnostic_sample_rate_rejects_nan_and_out_of_range() {
    let mut config = SignalConfig::default();
    assert!(config.validate().is_ok());

    config.observability.diagnostic_sample_rate = f64::NAN;
    assert!(config.validate().is_err());

    config.observability.diagnostic_sample_rate = f64::INFINITY;
    assert!(config.validate().is_err());

    config.observability.diagnostic_sample_rate = -1.0;
    assert!(config.validate().is_err());

    config.observability.diagnostic_sample_rate = 1.5;
    assert!(config.validate().is_err());

    config.observability.diagnostic_sample_rate = 0.5;
    assert!(config.validate().is_ok());
}

#[test]
fn generated_ids_use_uuidv7() -> Result<(), SignalError> {
    let id1 = MessageId::generate();
    let id2 = MessageId::generate();
    assert_ne!(id1.as_uuid(), id2.as_uuid());
    let version = id1
        .as_uuid()
        .get_version()
        .ok_or_else(|| SignalError::new(SignalErrorKind::Internal, "uuid version missing"))?;
    assert_eq!(version, uuid::Version::SortRand);
    Ok(())
}

#[test]
fn log_format_deserialization_is_case_insensitive_and_bounded() {
    let config: SignalConfig = toml::from_str("[observability]\nlog_format = \"json\"").unwrap();
    assert_eq!(config.observability.log_format, LogFormat::Json);

    let config: SignalConfig = toml::from_str("[observability]\nlog_format = \"JSON\"").unwrap();
    assert_eq!(config.observability.log_format, LogFormat::Json);

    let config: SignalConfig = toml::from_str("[observability]\nlog_format = \"compact\"").unwrap();
    assert_eq!(config.observability.log_format, LogFormat::Compact);

    let config: SignalConfig =
        toml::from_str("[observability]\nlog_format = \" COMPACT \"").unwrap();
    assert_eq!(config.observability.log_format, LogFormat::Compact);

    let config: SignalConfig = toml::from_str("[observability]\nlog_format = \"\"").unwrap();
    assert_eq!(config.observability.log_format, LogFormat::Json);

    let invalid = toml::from_str::<SignalConfig>("[observability]\nlog_format = \"verbose\"");
    assert!(invalid.is_err(), "expected invalid log format to fail");

    let long = "x".repeat(65);
    let oversized =
        toml::from_str::<SignalConfig>(&format!("[observability]\nlog_format = \"{long}\""));
    assert!(oversized.is_err(), "expected oversized log format to fail");
}

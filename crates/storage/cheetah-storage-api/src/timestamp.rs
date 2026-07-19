//! Timestamp conversion helpers shared by storage backends.

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::StorageError;

fn nanos_since_epoch(t: SystemTime) -> Result<i128, StorageError> {
    if let Ok(d) = t.duration_since(UNIX_EPOCH) {
        Ok(d.as_nanos() as i128)
    } else {
        let d = UNIX_EPOCH
            .duration_since(t)
            .map_err(|_| StorageError::internal("system time is ambiguous"))?;
        Ok(-(d.as_nanos() as i128))
    }
}

/// Converts a [`SystemTime`] to an [`OffsetDateTime`] without clamping.
pub fn system_time_to_offset(t: SystemTime) -> Result<OffsetDateTime, StorageError> {
    let nanos = nanos_since_epoch(t)?;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|e| StorageError::internal(format!("system time out of range: {e}")))
}

/// Converts an [`OffsetDateTime`] to a [`SystemTime`] without clamping.
pub fn offset_to_system_time(dt: OffsetDateTime) -> Result<SystemTime, StorageError> {
    let nanos = dt.unix_timestamp_nanos();
    let nanos_abs = nanos.checked_abs().map(|v| v as u128).unwrap_or(u128::MAX);
    let whole_seconds_u128 = nanos_abs / 1_000_000_000;
    let subsec_nanos = (nanos_abs % 1_000_000_000) as u32;
    let whole_seconds = u64::try_from(whole_seconds_u128)
        .map_err(|_| StorageError::internal("offset datetime too far from the Unix epoch"))?;
    let d = Duration::new(whole_seconds, subsec_nanos);

    if nanos < 0 {
        UNIX_EPOCH
            .checked_sub(d)
            .ok_or_else(|| StorageError::internal("offset datetime too far in the past"))
    } else {
        UNIX_EPOCH
            .checked_add(d)
            .ok_or_else(|| StorageError::internal("offset datetime too far in the future"))
    }
}

/// Formats a [`SystemTime`] as RFC 3339 without clamping.
pub fn system_time_to_rfc3339(t: SystemTime) -> Result<String, StorageError> {
    let dt = system_time_to_offset(t)?;
    dt.format(&Rfc3339)
        .map_err(|e| StorageError::internal(format!("failed to format RFC3339 timestamp: {e}")))
}

/// Parses an RFC 3339 string into a [`SystemTime`] without clamping.
pub fn rfc3339_to_system_time(s: &str) -> Result<SystemTime, StorageError> {
    let dt = OffsetDateTime::parse(s, &Rfc3339)
        .map_err(|e| StorageError::backend(format!("invalid RFC3339 timestamp: {e}")))?;
    offset_to_system_time(dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_system_time_to_offset() {
        let now = SystemTime::now();
        assert!(matches!(
            system_time_to_offset(now)
                .and_then(offset_to_system_time)
                .map(|back| back == now),
            Ok(true)
        ));
    }

    #[test]
    fn round_trip_rfc3339() {
        let now = SystemTime::now();
        assert!(matches!(
            system_time_to_rfc3339(now)
                .and_then(|s| rfc3339_to_system_time(&s))
                .map(|back| back == now),
            Ok(true)
        ));
    }

    #[test]
    fn pre_epoch_system_time_round_trips() {
        let t = UNIX_EPOCH - Duration::from_secs(1);
        assert!(matches!(
            system_time_to_rfc3339(t)
                .and_then(|s| rfc3339_to_system_time(&s))
                .map(|back| back == t),
            Ok(true)
        ));
    }

    #[test]
    fn far_future_round_trips() {
        let parsed = OffsetDateTime::parse("9999-12-31T23:59:59.999999999Z", &Rfc3339);
        assert!(parsed.is_ok());
        let dt = parsed.unwrap_or(OffsetDateTime::UNIX_EPOCH);
        if let Ok(t) = offset_to_system_time(dt) {
            assert!(matches!(system_time_to_offset(t), Ok(back) if back == dt));
        } else {
            panic!("offset_to_system_time failed for far-future date");
        }
    }

    #[test]
    fn far_past_round_trips() {
        let parsed = time::Date::from_calendar_date(-9999, time::Month::January, 1);
        assert!(parsed.is_ok());
        let date = parsed.unwrap_or(time::OffsetDateTime::UNIX_EPOCH.date());
        let dt =
            time::OffsetDateTime::new_in_offset(date, time::Time::MIDNIGHT, time::UtcOffset::UTC);
        if let Ok(t) = offset_to_system_time(dt) {
            assert!(matches!(system_time_to_offset(t), Ok(back) if back == dt));
        } else {
            panic!("offset_to_system_time failed for far-past date");
        }
    }
}

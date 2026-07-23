//! Time, duration and deadline types.

use crate::error::{Result, SignalError, SignalErrorKind};
use std::fmt;
use std::str::FromStr;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A UTC timestamp.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct UtcTimestamp(OffsetDateTime);

impl UtcTimestamp {
    /// Creates a timestamp from an existing UTC offset datetime.
    pub fn from_offset(value: OffsetDateTime) -> Self {
        debug_assert_eq!(value.offset(), time::UtcOffset::UTC);
        Self(value.to_offset(time::UtcOffset::UTC))
    }

    /// Creates a timestamp from milliseconds since the Unix epoch.
    ///
    /// Values outside the representable `OffsetDateTime` range are clamped to
    /// year -9999 or 9999 instead of panicking, so corrupt or extreme database
    /// rows do not crash the process.
    pub fn from_epoch_millis_saturating(ms: i64) -> Self {
        Self::from_offset(
            OffsetDateTime::UNIX_EPOCH
                .checked_add(time::Duration::milliseconds(ms))
                .unwrap_or_else(|| {
                    if ms < 0 {
                        OffsetDateTime::new_in_offset(
                            time::Date::MIN,
                            time::Time::MIDNIGHT,
                            time::UtcOffset::UTC,
                        )
                    } else {
                        OffsetDateTime::new_in_offset(
                            time::Date::MAX,
                            time::Time::MAX,
                            time::UtcOffset::UTC,
                        )
                    }
                }),
        )
    }

    /// Parses an RFC 3339 string into a timestamp.
    pub fn parse_rfc3339(value: &str) -> Result<Self> {
        let value = OffsetDateTime::parse(value, &Rfc3339).map_err(|e| {
            SignalError::new(SignalErrorKind::InvalidArgument, "invalid timestamp").with_source(e)
        })?;
        Ok(Self::from_offset(value))
    }

    /// Returns the underlying offset datetime in UTC.
    pub fn as_offset(self) -> OffsetDateTime {
        self.0
    }

    /// Returns the number of seconds since the Unix epoch.
    pub fn as_unix_seconds(self) -> i64 {
        self.0.unix_timestamp()
    }

    /// Adds a duration to this timestamp, returning `None` on overflow.
    pub fn checked_add(self, duration: DurationMs) -> Option<Self> {
        self.0
            .checked_add(duration.as_duration())
            .map(Self::from_offset)
    }

    /// Subtracts a duration from this timestamp, returning `None` on overflow.
    pub fn checked_sub(self, duration: DurationMs) -> Option<Self> {
        self.0
            .checked_sub(duration.as_duration())
            .map(Self::from_offset)
    }

    /// Returns the timestamp formatted as RFC 3339.
    pub fn to_rfc3339(self) -> Result<String> {
        self.0.format(&Rfc3339).map_err(|e| {
            SignalError::new(SignalErrorKind::Internal, "failed to format timestamp").with_source(e)
        })
    }

    /// Converts the timestamp to a `prost_types::Timestamp`.
    pub fn to_prost_timestamp(self) -> prost_types::Timestamp {
        prost_types::Timestamp {
            seconds: self.0.unix_timestamp(),
            nanos: self.0.nanosecond() as i32,
        }
    }

    /// Creates a timestamp from a `prost_types::Timestamp` if it is valid.
    pub fn from_prost_timestamp(ts: &prost_types::Timestamp) -> Option<Self> {
        let date = OffsetDateTime::from_unix_timestamp(ts.seconds).ok()?;
        let date = date + time::Duration::nanoseconds(ts.nanos as i64);
        Some(Self::from_offset(date))
    }
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_rfc3339() {
            Ok(s) => f.write_str(&s),
            Err(_) => Err(fmt::Error),
        }
    }
}

impl FromStr for UtcTimestamp {
    type Err = crate::error::SignalError;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse_rfc3339(s)
    }
}

impl Default for UtcTimestamp {
    fn default() -> Self {
        Self(OffsetDateTime::UNIX_EPOCH)
    }
}

/// A duration in milliseconds.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct DurationMs(i64);

impl DurationMs {
    /// Creates a duration from a number of milliseconds.
    pub const fn from_millis(value: i64) -> Self {
        Self(value)
    }

    /// Creates a duration from a number of seconds.
    pub const fn from_seconds(value: i64) -> Self {
        Self(value.saturating_mul(1_000))
    }

    /// Creates a duration from a number of minutes.
    pub const fn from_minutes(value: i64) -> Self {
        Self(value.saturating_mul(60_000))
    }

    /// Returns the duration as milliseconds.
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Returns the duration as a `time::Duration`.
    pub const fn as_duration(self) -> time::Duration {
        time::Duration::milliseconds(self.0)
    }

    /// Multiplies the duration by a scalar, returning `None` on overflow.
    pub const fn checked_mul(self, scalar: i32) -> Option<Self> {
        match self.0.checked_mul(scalar as i64) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Divides the duration by a scalar, returning `None` on division by zero.
    pub const fn checked_div(self, scalar: i32) -> Option<Self> {
        if scalar == 0 {
            None
        } else {
            Some(Self(self.0 / (scalar as i64)))
        }
    }
}

impl Default for DurationMs {
    fn default() -> Self {
        Self::from_millis(0)
    }
}

impl From<i64> for DurationMs {
    fn from(value: i64) -> Self {
        Self::from_millis(value)
    }
}

impl From<time::Duration> for DurationMs {
    fn from(value: time::Duration) -> Self {
        let ms = value.whole_milliseconds();
        let clamped = if ms > i64::MAX as i128 {
            i64::MAX
        } else if ms < i64::MIN as i128 {
            i64::MIN
        } else {
            ms as i64
        };
        Self::from_millis(clamped)
    }
}

impl From<DurationMs> for time::Duration {
    fn from(value: DurationMs) -> Self {
        value.as_duration()
    }
}

/// A point in time by which an operation must complete.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Deadline(UtcTimestamp);

impl Deadline {
    /// Creates a deadline from a wall timestamp.
    pub fn from_timestamp(timestamp: UtcTimestamp) -> Self {
        Self(timestamp)
    }

    /// Computes a deadline from a clock and a duration.
    pub fn from_now(now: UtcTimestamp, duration: DurationMs) -> Option<Self> {
        now.checked_add(duration).map(Self::from_timestamp)
    }

    /// Returns the timestamp at which the deadline expires.
    pub fn as_timestamp(self) -> UtcTimestamp {
        self.0
    }

    /// Returns whether the deadline has passed relative to `now`.
    pub fn is_elapsed(self, now: UtcTimestamp) -> bool {
        now >= self.0
    }

    /// Returns the remaining time until the deadline, or `None` if already elapsed.
    pub fn remaining(self, now: UtcTimestamp) -> Option<DurationMs> {
        if self.is_elapsed(now) {
            None
        } else {
            let diff =
                self.0.as_offset().unix_timestamp_nanos() - now.as_offset().unix_timestamp_nanos();
            let diff_millis = diff / 1_000_000;
            let diff_millis_i64 = if diff_millis > i64::MAX as i128 {
                i64::MAX
            } else {
                diff_millis as i64
            };
            Some(DurationMs::from_millis(diff_millis_i64))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn from_time_duration_clamps_to_i64_range() {
        // Seconds are multiplied to milliseconds, producing an i128 value outside the i64 range.
        let huge = time::Duration::seconds(i64::MAX);
        let d = DurationMs::from(huge);
        assert_eq!(d.as_millis(), i64::MAX);

        let negative_huge = time::Duration::seconds(i64::MIN);
        let d = DurationMs::from(negative_huge);
        assert_eq!(d.as_millis(), i64::MIN);
    }

    #[test]
    fn from_seconds_saturates_on_overflow() {
        assert_eq!(DurationMs::from_seconds(i64::MAX).as_millis(), i64::MAX);
        assert_eq!(DurationMs::from_seconds(i64::MIN).as_millis(), i64::MIN);
    }

    #[test]
    fn from_epoch_millis_saturating_clamps_out_of_range_values() {
        // Values within the representable range round-trip through epoch ms.
        let ts = UtcTimestamp::from_epoch_millis_saturating(60_000);
        assert_eq!(ts.as_offset().unix_timestamp() * 1000, 60_000);

        // `i64::MAX` ms overflows `OffsetDateTime` and must clamp to year 9999
        // instead of panicking.
        let far = UtcTimestamp::from_epoch_millis_saturating(i64::MAX);
        assert_eq!(far.as_offset().year(), 9999);

        // `i64::MIN` ms underflows and must clamp to year -9999.
        let past = UtcTimestamp::from_epoch_millis_saturating(i64::MIN);
        assert_eq!(past.as_offset().year(), -9999);
    }
}

//! ONVIF `GetSystemDateAndTime` request builder and response parser.
//!
//! This is the first unauthenticated call used to discover device clock offset.

use crate::error::{OnvifError, OnvifResult};
use crate::soap::Envelope;
use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::events::{BytesStart, Event};
use std::io::Cursor;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

const ACTION: &str = "http://www.onvif.org/ver10/device/wsdl/GetSystemDateAndTime";
const DEVICE_NS: &str = "http://www.onvif.org/ver10/device/wsdl";

/// A calendar date and time with second precision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateTime {
    /// Year, e.g. 2026.
    pub year: i32,
    /// 1-12.
    pub month: u8,
    /// 1-31.
    pub day: u8,
    /// 0-23.
    pub hour: u8,
    /// 0-59.
    pub minute: u8,
    /// 0-59.
    pub second: u8,
}

impl DateTime {
    /// Constructs a `time::PrimitiveDateTime` from these components.
    ///
    /// Returns an error if any component is out of range.
    pub fn to_primitive(&self) -> OnvifResult<PrimitiveDateTime> {
        let month = Month::try_from(self.month).map_err(|_| {
            OnvifError::invalid_field(format!("DateTime month out of range: {}", self.month))
        })?;
        let date = Date::from_calendar_date(self.year, month, self.day).map_err(|_| {
            OnvifError::invalid_field(format!(
                "DateTime date out of range: {}-{}-{}",
                self.year, self.month, self.day
            ))
        })?;
        let time = Time::from_hms(self.hour, self.minute, self.second).map_err(|_| {
            OnvifError::invalid_field(format!(
                "DateTime time out of range: {}:{}:{}",
                self.hour, self.minute, self.second
            ))
        })?;
        Ok(PrimitiveDateTime::new(date, time))
    }

    /// Assumes UTC and returns an `OffsetDateTime`.
    pub fn to_utc(&self) -> OnvifResult<OffsetDateTime> {
        Ok(self.to_primitive()?.assume_utc())
    }
}

/// Parsed `GetSystemDateAndTimeResponse`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemDateAndTime {
    /// `Manual`, `NTP` or similar value from the device.
    pub date_time_type: String,
    /// Whether daylight savings is active.
    pub daylight_savings: bool,
    /// Optional `TimeZone/TZ` string.
    pub timezone: Option<String>,
    /// Device time in UTC.
    pub utc: DateTime,
    /// Optional device local time.
    pub local: Option<DateTime>,
}

/// Builds the SOAP body for an unauthenticated `GetSystemDateAndTime` request.
pub fn build_get_system_date_and_time(message_id: impl Into<String>) -> OnvifResult<String> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);

    let mut body = BytesStart::new("tds:GetSystemDateAndTime");
    body.push_attribute(("xmlns:tds", DEVICE_NS));
    writer.write_event(Event::Empty(body))?;

    let body = String::from_utf8(cursor.into_inner()).map_err(OnvifError::from)?;
    Envelope::new(ACTION, body)
        .with_message_id(message_id)
        .build()
}

/// Parses the `GetSystemDateAndTimeResponse` SOAP body.
pub fn parse_get_system_date_and_time_response(xml: &str) -> OnvifResult<SystemDateAndTime> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut date_time_type: Option<String> = None;
    let mut daylight_savings: Option<bool> = None;
    let mut timezone: Option<String> = None;
    let mut utc: Option<DateTime> = None;
    let mut local: Option<DateTime> = None;

    let mut context: Vec<String> = Vec::new();
    let mut utc_builder = DateTimeBuilder::default();
    let mut local_builder = DateTimeBuilder::default();
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                context.push(local_name(&e.name()));
                text.clear();
            }
            Ok(Event::Empty(_e)) => {
                // Empty elements are leaves; do not push onto the context stack
                // because there will be no matching End event to pop them.
                text.clear();
            }
            Ok(Event::Text(e)) => {
                text.push_str(&e.xml10_content().unwrap_or_default());
            }
            Ok(Event::End(e)) => {
                let name = local_name(&e.name());
                let current = context.last().map(String::as_str);
                let in_utc = context.iter().any(|s| s == "UTCDateTime");
                let in_local = context.iter().any(|s| s == "LocalDateTime");

                if current == Some("DateTimeType") {
                    date_time_type = Some(text.trim().to_string());
                } else if current == Some("DaylightSavings") {
                    daylight_savings = Some(text.trim().eq_ignore_ascii_case("true"));
                } else if current == Some("TZ") {
                    timezone = Some(text.trim().to_string());
                } else if current == Some("Hour") && in_utc {
                    utc_builder.hour = parse_component(&text, "hour")?;
                } else if current == Some("Minute") && in_utc {
                    utc_builder.minute = parse_component(&text, "minute")?;
                } else if current == Some("Second") && in_utc {
                    utc_builder.second = parse_component(&text, "second")?;
                } else if current == Some("Year") && in_utc {
                    utc_builder.year = parse_component(&text, "year")?;
                } else if current == Some("Month") && in_utc {
                    utc_builder.month = parse_component(&text, "month")?;
                } else if current == Some("Day") && in_utc {
                    utc_builder.day = parse_component(&text, "day")?;
                } else if current == Some("Hour") && in_local {
                    local_builder.hour = parse_component(&text, "hour")?;
                } else if current == Some("Minute") && in_local {
                    local_builder.minute = parse_component(&text, "minute")?;
                } else if current == Some("Second") && in_local {
                    local_builder.second = parse_component(&text, "second")?;
                } else if current == Some("Year") && in_local {
                    local_builder.year = parse_component(&text, "year")?;
                } else if current == Some("Month") && in_local {
                    local_builder.month = parse_component(&text, "month")?;
                } else if current == Some("Day") && in_local {
                    local_builder.day = parse_component(&text, "day")?;
                }

                if name == "UTCDateTime" {
                    utc = Some(utc_builder.build()?);
                    utc_builder = DateTimeBuilder::default();
                } else if name == "LocalDateTime" {
                    // LocalDateTime is optional; a malformed optional block is
                    // ignored rather than failing the whole response.
                    match local_builder.build() {
                        Ok(d) => local = Some(d),
                        Err(e) => {
                            tracing::warn!("ignoring malformed optional LocalDateTime: {e}");
                            local = None;
                        }
                    }
                    local_builder = DateTimeBuilder::default();
                }

                context.pop();
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(OnvifError::xml(e)),
            _ => {}
        }
    }

    let utc = utc.ok_or_else(|| OnvifError::missing_field("UTCDateTime".to_string()))?;
    let date_time_type =
        date_time_type.ok_or_else(|| OnvifError::missing_field("DateTimeType".to_string()))?;
    let daylight_savings = daylight_savings.unwrap_or(false);

    Ok(SystemDateAndTime {
        date_time_type,
        daylight_savings,
        timezone,
        utc,
        local,
    })
}

#[derive(Default)]
struct DateTimeBuilder {
    year: Option<i32>,
    month: Option<u8>,
    day: Option<u8>,
    hour: Option<u8>,
    minute: Option<u8>,
    second: Option<u8>,
}

impl DateTimeBuilder {
    fn build(self) -> OnvifResult<DateTime> {
        let date_time = DateTime {
            year: self.year.ok_or_else(|| missing("year"))?,
            month: self.month.ok_or_else(|| missing("month"))?,
            day: self.day.ok_or_else(|| missing("day"))?,
            hour: self.hour.ok_or_else(|| missing("hour"))?,
            minute: self.minute.ok_or_else(|| missing("minute"))?,
            second: self.second.ok_or_else(|| missing("second"))?,
        };
        // Validate that the parsed components form a real calendar date/time.
        date_time.to_primitive()?;
        Ok(date_time)
    }
}

fn missing(field: &str) -> OnvifError {
    OnvifError::missing_field(format!("DateTime/{field}"))
}

fn parse_component<T: std::str::FromStr>(text: &str, name: &str) -> OnvifResult<Option<T>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<T>().map(Some).map_err(|_| {
        OnvifError::invalid_field(format!("DateTime/{name} is not a valid integer: {text}"))
    })
}

fn local_name(name: &quick_xml::name::QName<'_>) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn builds_unauthenticated_request() {
        let xml = build_get_system_date_and_time("urn:uuid:1").unwrap();
        assert!(xml.contains(ACTION));
        assert!(xml.contains("GetSystemDateAndTime"));
        assert!(xml.contains("urn:uuid:1"));
    }

    #[test]
    fn parses_response_with_utc_time() {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetSystemDateAndTimeResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:SystemDateAndTime>
        <tt:DateTimeType xmlns:tt="http://www.onvif.org/ver10/schema">NTP</tt:DateTimeType>
        <tt:DaylightSavings xmlns:tt="http://www.onvif.org/ver10/schema">false</tt:DaylightSavings>
        <tt:TimeZone xmlns:tt="http://www.onvif.org/ver10/schema"><tt:TZ>CST-8:00:00</tt:TZ></tt:TimeZone>
        <tt:UTCDateTime xmlns:tt="http://www.onvif.org/ver10/schema">
          <tt:Time><tt:Hour>14</tt:Hour><tt:Minute>31</tt:Minute><tt:Second>0</tt:Second></tt:Time>
          <tt:Date><tt:Year>2026</tt:Year><tt:Month>7</tt:Month><tt:Day>13</tt:Day></tt:Date>
        </tt:UTCDateTime>
      </tds:SystemDateAndTime>
    </tds:GetSystemDateAndTimeResponse>
  </s:Body>
</s:Envelope>"#;
        let res = parse_get_system_date_and_time_response(xml).unwrap();
        assert_eq!(res.date_time_type, "NTP");
        assert!(!res.daylight_savings);
        assert_eq!(res.timezone.as_deref(), Some("CST-8:00:00"));
        assert_eq!(res.utc.year, 2026);
        assert_eq!(res.utc.month, 7);
        assert_eq!(res.utc.day, 13);
        assert_eq!(res.utc.hour, 14);
        assert_eq!(res.utc.minute, 31);
        assert_eq!(res.utc.second, 0);
        let dt = res.utc.to_utc().unwrap();
        assert_eq!(dt.unix_timestamp(), 1_783_953_060);
    }

    #[test]
    fn parses_response_with_self_closing_extension() {
        // Some devices include empty <tt:Extension/> elements that must not
        // corrupt the parser's context stack.
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetSystemDateAndTimeResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:SystemDateAndTime>
        <tt:DateTimeType xmlns:tt="http://www.onvif.org/ver10/schema">NTP</tt:DateTimeType>
        <tt:DaylightSavings xmlns:tt="http://www.onvif.org/ver10/schema">false</tt:DaylightSavings>
        <tt:Extension xmlns:tt="http://www.onvif.org/ver10/schema"/>
        <tt:UTCDateTime xmlns:tt="http://www.onvif.org/ver10/schema">
          <tt:Time><tt:Hour>14</tt:Hour><tt:Minute>31</tt:Minute><tt:Second>0</tt:Second></tt:Time>
          <tt:Date><tt:Year>2026</tt:Year><tt:Month>7</tt:Month><tt:Day>13</tt:Day></tt:Date>
        </tt:UTCDateTime>
      </tds:SystemDateAndTime>
    </tds:GetSystemDateAndTimeResponse>
  </s:Body>
</s:Envelope>"#;
        let res = parse_get_system_date_and_time_response(xml).unwrap();
        assert_eq!(res.utc.year, 2026);
    }

    #[test]
    fn rejects_invalid_date_time_components() {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetSystemDateAndTimeResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:SystemDateAndTime>
        <tt:DateTimeType xmlns:tt="http://www.onvif.org/ver10/schema">NTP</tt:DateTimeType>
        <tt:DaylightSavings xmlns:tt="http://www.onvif.org/ver10/schema">false</tt:DaylightSavings>
        <tt:UTCDateTime xmlns:tt="http://www.onvif.org/ver10/schema">
          <tt:Time><tt:Hour>14</tt:Hour><tt:Minute>31</tt:Minute><tt:Second>0</tt:Second></tt:Time>
          <tt:Date><tt:Year>2026</tt:Year><tt:Month>13</tt:Month><tt:Day>13</tt:Day></tt:Date>
        </tt:UTCDateTime>
      </tds:SystemDateAndTime>
    </tds:GetSystemDateAndTimeResponse>
  </s:Body>
</s:Envelope>"#;
        let result = parse_get_system_date_and_time_response(xml);
        assert!(
            result.is_err(),
            "parser must reject out-of-range date/time components, got {:?}",
            result
        );
    }

    #[test]
    fn rejects_non_numeric_date_time_component() {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetSystemDateAndTimeResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:SystemDateAndTime>
        <tt:DateTimeType xmlns:tt="http://www.onvif.org/ver10/schema">NTP</tt:DateTimeType>
        <tt:DaylightSavings xmlns:tt="http://www.onvif.org/ver10/schema">false</tt:DaylightSavings>
        <tt:UTCDateTime xmlns:tt="http://www.onvif.org/ver10/schema">
          <tt:Time><tt:Hour>not-a-number</tt:Hour><tt:Minute>31</tt:Minute><tt:Second>0</tt:Second></tt:Time>
          <tt:Date><tt:Year>2026</tt:Year><tt:Month>7</tt:Month><tt:Day>13</tt:Day></tt:Date>
        </tt:UTCDateTime>
      </tds:SystemDateAndTime>
    </tds:GetSystemDateAndTimeResponse>
  </s:Body>
</s:Envelope>"#;
        let result = parse_get_system_date_and_time_response(xml);
        assert!(
            result.is_err(),
            "parser must reject non-numeric date/time components, got {:?}",
            result
        );
    }
}

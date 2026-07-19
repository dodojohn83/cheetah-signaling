//! GB28181 access module parsing helpers.

use crate::config::Gb28181DomainConfig;
use crate::error::AccessError;
use crate::types::DeviceId;
use cheetah_gb28181_core::{DigestResponse, HeaderName, SipHeaders, SipUri};

pub(crate) fn device_id_from_request(
    request: &cheetah_gb28181_core::RequestLine,
    headers: &SipHeaders,
) -> Result<DeviceId, AccessError> {
    if let Some(id) = request
        .uri
        .user()
        .filter(|u| !u.is_empty())
        .and_then(DeviceId::new)
    {
        return Ok(id);
    }
    if let Some(id) = headers
        .get(&HeaderName::To)
        .and_then(|v| device_from_address(v.as_str()))
    {
        return Ok(id);
    }
    if let Some(id) = headers
        .get(&HeaderName::From)
        .and_then(|v| device_from_address(v.as_str()))
    {
        return Ok(id);
    }
    Err(AccessError::InvalidDeviceId)
}

pub(crate) fn device_from_address(value: &str) -> Option<DeviceId> {
    let value = value.trim();
    let uri_text = if let Some(start) = value.find('<') {
        let end = value.find('>')?;
        value.get(start + 1..end)?
    } else {
        value.split(';').next()?
    };
    SipUri::parse(uri_text).ok().and_then(|u| {
        u.user()
            .filter(|u| !u.is_empty())
            .map(str::to_string)
            .and_then(DeviceId::new)
    })
}

pub(crate) fn parse_contact_header(
    headers: &SipHeaders,
) -> Result<(SipUri, Option<u32>), AccessError> {
    let value = headers
        .get(&HeaderName::Contact)
        .ok_or(AccessError::InvalidContact)?
        .as_str();
    parse_address_with_expires(value)
}

pub(crate) fn parse_address_with_expires(
    value: &str,
) -> Result<(SipUri, Option<u32>), AccessError> {
    let value = value.trim();
    let (uri_text, params_text) = if let Some(start) = value.find('<') {
        let end = value.find('>').ok_or(AccessError::InvalidContact)?;
        let uri_text = value
            .get(start + 1..end)
            .ok_or(AccessError::InvalidContact)?;
        let after = value.get(end + 1..).unwrap_or("");
        (uri_text, after.trim())
    } else {
        let parts: Vec<&str> = value.splitn(2, ';').collect();
        (parts[0].trim(), parts.get(1).copied().unwrap_or(""))
    };

    let uri = SipUri::parse(uri_text).map_err(|_| AccessError::InvalidContact)?;
    let mut expires = None;
    for token in params_text.split(';') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(value) = token.strip_prefix("expires=") {
            let value = value.trim();
            if value.is_empty() {
                return Err(AccessError::InvalidExpires);
            }
            expires = Some(
                value
                    .parse::<u32>()
                    .map_err(|_| AccessError::InvalidExpires)?,
            );
        }
    }
    Ok((uri, expires))
}

pub(crate) fn parse_expires_header(headers: &SipHeaders) -> Result<Option<u32>, AccessError> {
    let Some(value) = headers.get(&HeaderName::Expires) else {
        return Ok(None);
    };
    let trimmed = value.as_str().trim();
    if trimmed.is_empty() {
        return Err(AccessError::InvalidExpires);
    }
    trimmed
        .parse::<u32>()
        .map(Some)
        .map_err(|_| AccessError::InvalidExpires)
}

pub(crate) fn resolve_expires(
    contact_expires: Option<u32>,
    header_expires: Option<u32>,
    config: &Gb28181DomainConfig,
) -> u32 {
    let requested = contact_expires
        .or(header_expires)
        .unwrap_or(config.default_expires_seconds());
    requested.clamp(0, config.max_expires_seconds())
}

pub(crate) fn parse_authorization(
    value: &str,
) -> Result<DigestResponse, cheetah_gb28181_core::DigestError> {
    DigestResponse::parse_with_limit(value, 4096)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use cheetah_gb28181_core::{HeaderName, HeaderValue, SipHeaders};

    #[test]
    fn parse_expires_header_rejects_non_numeric() {
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::Expires, HeaderValue::new("not-a-number"));
        assert!(matches!(
            parse_expires_header(&headers),
            Err(AccessError::InvalidExpires)
        ));
    }

    #[test]
    fn parse_expires_header_rejects_empty() {
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::Expires, HeaderValue::new(""));
        assert!(matches!(
            parse_expires_header(&headers),
            Err(AccessError::InvalidExpires)
        ));
    }

    #[test]
    fn parse_address_with_expires_rejects_non_numeric_param() {
        let result = parse_address_with_expires("<sip:a@example.com>;expires=not-a-number");
        assert!(matches!(result, Err(AccessError::InvalidExpires)));
    }

    #[test]
    fn parse_address_with_expires_accepts_valid_param() {
        let (uri, expires) = parse_address_with_expires("<sip:a@example.com>;expires=60").unwrap();
        assert_eq!(expires, Some(60));
        assert_eq!(uri.user(), Some("a"));
    }
}

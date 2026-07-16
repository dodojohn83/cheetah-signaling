//! GB28181 device and channel identifier validation.

use crate::Gb28181ModuleError;

/// Maximum length of a GB28181 device ID in characters.
pub const MAX_DEVICE_ID_LEN: usize = 64;

/// Validates a device identifier for use in `From`/`Contact` URIs.
///
/// GB/T 28181 device IDs are normally 20 decimal digits, but the module
/// allows configurable rules through the parent `Gb28181Config`. The helper
/// here rejects empty strings, NUL and obvious injection characters.
pub fn validate_device_id(value: &str) -> Result<String, Gb28181ModuleError> {
    if value.is_empty() {
        return Err(Gb28181ModuleError::InvalidDeviceId(
            "device id is empty".into(),
        ));
    }
    if value.len() > MAX_DEVICE_ID_LEN {
        return Err(Gb28181ModuleError::InvalidDeviceId(format!(
            "device id exceeds {MAX_DEVICE_ID_LEN} characters"
        )));
    }
    if value.contains('\0') || value.contains('\r') || value.contains('\n') {
        return Err(Gb28181ModuleError::InvalidDeviceId(
            "device id contains control characters".into(),
        ));
    }
    Ok(value.to_string())
}

/// Extracts the user part of a `sip:user@host` style string without validating
/// the host part. Used to retrieve the device ID from `From`/`To` headers.
pub fn extract_uri_user(raw: &str) -> Option<&str> {
    let mut raw = raw.trim();
    raw = raw.strip_prefix('<').unwrap_or(raw);
    raw = raw.strip_suffix('>').unwrap_or(raw);
    raw = raw.strip_prefix("sip:").unwrap_or(raw);
    raw = raw.strip_prefix("sips:").unwrap_or(raw);
    let user = raw.rsplit_once('@').map_or(raw, |(user, _)| user);
    Some(user.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_long_and_control_chars() {
        assert!(validate_device_id("").is_err());
        assert!(validate_device_id("a\r\nb").is_err());
        assert!(validate_device_id("\0").is_err());
        assert!(validate_device_id(&"1".repeat(65)).is_err());
        assert_eq!(
            validate_device_id("34020000001320000001").unwrap_or_else(|_| "invalid".to_string()),
            "34020000001320000001"
        );
    }

    #[test]
    fn extracts_user_part() {
        assert_eq!(
            extract_uri_user("<sip:34020000001320000001@3402000000>"),
            Some("34020000001320000001")
        );
        assert_eq!(
            extract_uri_user("34020000001320000001@3402000000"),
            Some("34020000001320000001")
        );
        assert_eq!(
            extract_uri_user("sip:34020000001320000001@3402000000"),
            Some("34020000001320000001")
        );
    }
}

//! WS-Security UsernameToken injection for outbound SOAP envelopes.

use crate::error::{DriverError, DriverResult};
use cheetah_onvif_core::security::{ClockOffset, NONCE_SIZE, UsernameToken};
use rand::RngCore;
use secrecy::SecretString;
use std::time::{SystemTime, UNIX_EPOCH};

/// Credentials for an ONVIF device endpoint.
#[derive(Clone, Debug)]
pub struct DeviceCredentials {
    /// Username.
    pub username: String,
    /// Password (zeroized via SecretString).
    pub password: SecretString,
    /// Use PasswordText instead of PasswordDigest (compatibility only).
    pub password_text: bool,
    /// Device clock offset in seconds (from GetSystemDateAndTime).
    pub clock_offset_seconds: i64,
}

/// Injects a `wsse:Security` UsernameToken into an existing SOAP envelope XML.
///
/// The token is inserted immediately before the closing `</s:Header>` /
/// `</Header>` tag. If no header is present, returns an error so callers do not
/// silently send unauthenticated requests.
pub fn inject_username_token(
    envelope_xml: &str,
    credentials: &DeviceCredentials,
    now_unix_seconds: Option<i64>,
) -> DriverResult<String> {
    let created_seconds = now_unix_seconds.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });

    let mut nonce = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut nonce);

    let offset = ClockOffset(credentials.clock_offset_seconds);
    let token = if credentials.password_text {
        UsernameToken::text(
            credentials.username.clone(),
            credentials.password.clone(),
            nonce,
            created_seconds,
            offset,
        )
    } else {
        UsernameToken::digest(
            credentials.username.clone(),
            credentials.password.clone(),
            nonce,
            created_seconds,
            offset,
        )
    };

    let token_xml = token.to_xml().map_err(DriverError::Onvif)?;
    let security = format!(
        r#"<wsse:Security xmlns:wsse="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd" s:mustUnderstand="1">{token_xml}</wsse:Security>"#
    );

    // Prefer SOAP 1.2 prefix used by this codebase.
    if let Some(idx) = envelope_xml.find("</s:Header>") {
        let mut out = String::with_capacity(envelope_xml.len() + security.len());
        out.push_str(&envelope_xml[..idx]);
        out.push_str(&security);
        out.push_str(&envelope_xml[idx..]);
        return Ok(out);
    }
    if let Some(idx) = envelope_xml.find("</Header>") {
        let mut out = String::with_capacity(envelope_xml.len() + security.len());
        out.push_str(&envelope_xml[..idx]);
        out.push_str(&security);
        out.push_str(&envelope_xml[idx..]);
        return Ok(out);
    }

    Err(DriverError::Config(
        "SOAP envelope has no Header element for UsernameToken injection".into(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use secrecy::SecretString;

    #[test]
    fn injects_before_header_close() {
        let envelope = r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Header><a:Action>x</a:Action></s:Header><s:Body/></s:Envelope>"#;
        let creds = DeviceCredentials {
            username: "admin".into(),
            password: SecretString::from("secret".to_string()),
            password_text: false,
            clock_offset_seconds: 0,
        };
        let out = inject_username_token(envelope, &creds, Some(1_700_000_000))
            .expect("username token injection should succeed");
        assert!(out.contains("wsse:UsernameToken"));
        assert!(out.contains("admin"));
        assert!(!out.contains("secret"));
        let security_pos = out
            .find("wsse:Security")
            .expect("wsse:Security should be present");
        let header_close_pos = out
            .find("</s:Header>")
            .expect("</s:Header> close tag should be present");
        assert!(security_pos < header_close_pos);
    }
}

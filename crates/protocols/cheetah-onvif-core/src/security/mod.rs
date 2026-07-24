//! WS-Security `UsernameToken` helpers for ONVIF SOAP requests.

use crate::error::{OnvifError, OnvifResult};
use secrecy::{ExposeSecret, SecretString};
use sha1::{Digest, Sha1};

/// Number of random bytes for a WS-Security nonce.
pub const NONCE_SIZE: usize = 16;

/// ONVIF `UsernameToken` carrying either a `PasswordDigest` or `PasswordText`.
///
/// The nonce and the `Created` timestamp are supplied by the caller. This keeps
/// the core crate free of global random and clock sources so the driver can
/// inject deterministic values in tests and calibrated device time in production.
#[derive(Clone, Debug)]
pub struct UsernameToken {
    username: String,
    password: SecretString,
    nonce: [u8; NONCE_SIZE],
    created: String,
    kind: TokenKind,
    clock_offset: ClockOffset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenKind {
    Digest,
    Text,
}

/// A calibrated or discovered device clock offset in seconds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClockOffset(pub i64);

impl UsernameToken {
    /// Creates a `PasswordDigest` token.
    ///
    /// `created_seconds` is the local Unix timestamp; `clock_offset` is added
    /// before formatting so the token reflects device time.
    pub fn digest(
        username: impl Into<String>,
        password: SecretString,
        nonce: [u8; NONCE_SIZE],
        created_seconds: i64,
        clock_offset: ClockOffset,
    ) -> Self {
        Self::new(
            username,
            password,
            nonce,
            created_seconds,
            clock_offset,
            TokenKind::Digest,
        )
    }

    /// Creates a `PasswordText` token for compatibility profiles that explicitly
    /// require it. Most deployments should prefer `digest`.
    pub fn text(
        username: impl Into<String>,
        password: SecretString,
        nonce: [u8; NONCE_SIZE],
        created_seconds: i64,
        clock_offset: ClockOffset,
    ) -> Self {
        Self::new(
            username,
            password,
            nonce,
            created_seconds,
            clock_offset,
            TokenKind::Text,
        )
    }

    fn new(
        username: impl Into<String>,
        password: SecretString,
        nonce: [u8; NONCE_SIZE],
        created_seconds: i64,
        clock_offset: ClockOffset,
        kind: TokenKind,
    ) -> Self {
        let offset_seconds = created_seconds.saturating_add(clock_offset.0);
        Self {
            username: username.into(),
            password,
            nonce,
            created: format_created(offset_seconds),
            kind,
            clock_offset,
        }
    }

    /// Returns the calibrated clock offset used when the token was created.
    pub fn clock_offset(&self) -> ClockOffset {
        self.clock_offset
    }

    /// Builds the `wsse:UsernameToken` XML fragment.
    pub fn to_xml(&self) -> OnvifResult<String> {
        let nonce_b64 = base64_encode(&self.nonce);
        let created = &self.created;
        let password = match self.kind {
            TokenKind::Digest => {
                let digest = password_digest(&self.nonce, created, self.password.expose_secret());
                base64_encode(&digest)
            }
            TokenKind::Text => self.password.expose_secret().to_string(),
        };
        let password_type = match self.kind {
            TokenKind::Digest => {
                "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest"
            }
            TokenKind::Text => {
                "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordText"
            }
        };

        let mut out = Vec::new();
        {
            use quick_xml::Writer;
            use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
            let mut writer = Writer::new(&mut out);

            let mut token = BytesStart::new("wsse:UsernameToken");
            token.push_attribute((
                "xmlns:wsse",
                "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd",
            ));
            token.push_attribute(("xmlns:wsu", "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd"));
            writer.write_event(Event::Start(token))?;

            writer.write_event(Event::Start(BytesStart::new("wsse:Username")))?;
            writer.write_event(Event::Text(BytesText::new(&self.username)))?;
            writer.write_event(Event::End(BytesEnd::new("wsse:Username")))?;

            let mut pw = BytesStart::new("wsse:Password");
            pw.push_attribute(("Type", password_type));
            writer.write_event(Event::Start(pw))?;
            writer.write_event(Event::Text(BytesText::new(&password)))?;
            writer.write_event(Event::End(BytesEnd::new("wsse:Password")))?;

            let mut nc = BytesStart::new("wsse:Nonce");
            nc.push_attribute(("EncodingType", "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary"));
            writer.write_event(Event::Start(nc))?;
            writer.write_event(Event::Text(BytesText::new(&nonce_b64)))?;
            writer.write_event(Event::End(BytesEnd::new("wsse:Nonce")))?;

            writer.write_event(Event::Start(BytesStart::new("wsu:Created")))?;
            writer.write_event(Event::Text(BytesText::new(created)))?;
            writer.write_event(Event::End(BytesEnd::new("wsu:Created")))?;

            writer.write_event(Event::End(BytesEnd::new("wsse:UsernameToken")))?;
        }
        String::from_utf8(out).map_err(OnvifError::xml)
    }
}

fn password_digest(nonce: &[u8], created: &str, password: &str) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(nonce);
    hasher.update(created.as_bytes());
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; 20];
    out.copy_from_slice(&result);
    out
}

fn base64_encode(input: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose};
    general_purpose::STANDARD.encode(input)
}

fn format_created(seconds: i64) -> String {
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    let dt = OffsetDateTime::from_unix_timestamp(seconds).unwrap_or(OffsetDateTime::UNIX_EPOCH);
    dt.format(&Rfc3339).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const NONCE: [u8; NONCE_SIZE] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ];

    #[test]
    fn digest_token_contains_expected_elements() {
        let token = UsernameToken::digest(
            "admin",
            SecretString::from("password"),
            NONCE,
            1700000000,
            ClockOffset(0),
        );
        let xml = token.to_xml().unwrap();
        assert!(xml.contains("<wsse:UsernameToken"));
        assert!(xml.contains("<wsse:Username>admin</wsse:Username>"));
        assert!(xml.contains("<wsse:Password"));
        assert!(xml.contains("PasswordDigest"));
        assert!(xml.contains("<wsse:Nonce"));
        assert!(xml.contains("<wsu:Created>"));
        // Standard PasswordDigest = Base64(SHA1(nonce_bytes + created + password))
        assert!(xml.contains(">/On4elqsNyvH1HVvpEVBb36lTmQ=<"));
    }

    #[test]
    fn text_token_uses_password_text_type() {
        let token = UsernameToken::text(
            "admin",
            SecretString::from("password"),
            NONCE,
            1700000000,
            ClockOffset(0),
        );
        let xml = token.to_xml().unwrap();
        assert!(xml.contains("PasswordText"));
        assert!(xml.contains("password"));
    }

    #[test]
    fn clock_offset_is_added_to_created() {
        let token = UsernameToken::digest(
            "admin",
            SecretString::from("password"),
            NONCE,
            1700000000,
            ClockOffset(60),
        );
        let xml = token.to_xml().unwrap();
        assert!(xml.contains("<wsu:Created>"));
        // 1700000000 + 60 = 2023-11-14T22:14:20+00:00
        assert!(xml.contains("2023-11-14T22:14:20"));
    }
}

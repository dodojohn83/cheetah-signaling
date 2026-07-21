//! Redaction helpers for logs, traces and audit events.
//!
//! `SafeDetails` wraps a string that has been passed through a conservative
//! redaction pass before being stored in an `AuditEvent` or emitted to logs.
//! The redactor only strips whole values of known sensitive SIP/XML fields; it
//! does not attempt to guess arbitrary secrets.

use std::fmt;

/// Patterns whose line/element contents are replaced with `[REDACTED]`.
const SENSITIVE_KEYS: &[&str] = &[
    "Authorization",
    "Proxy-Authorization",
    "WWW-Authenticate",
    "Authentication-Info",
    "password",
    "Password",
    "secret",
    "Secret",
    "nonce",
    "Nonce",
    "credentials",
    "Credentials",
    "token",
    "Token",
    "privateKey",
    "PrivateKey",
    "key",
    "Key",
];

fn contains_sensitive_key(s: &str) -> bool {
    SENSITIVE_KEYS.iter().any(|k| s.contains(k))
}

fn redact_line(line: &str) -> String {
    // SIP header-like: "Name: value" or "Name=value"
    if let Some(colon) = line.find(':') {
        let name = &line[..colon];
        if contains_sensitive_key(name) {
            return format!("{name}: [REDACTED]");
        }
    }
    if let Some(eq) = line.find('=') {
        let name = &line[..eq];
        if contains_sensitive_key(name) {
            return format!("{name}=[REDACTED]");
        }
    }
    if contains_sensitive_key(line) {
        return "[REDACTED]".to_string();
    }
    line.to_string()
}

/// Apply conservative redaction to a detail string.
pub fn redact_details(input: &str) -> String {
    input
        .lines()
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A string that has been through conservative redaction and is safe to log in
/// audit events and traces.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SafeDetails(pub String);

impl SafeDetails {
    /// Create a new `SafeDetails` by redacting `input`.
    pub fn new(input: impl Into<String>) -> Self {
        Self(redact_details(&input.into()))
    }

    /// Borrow the redacted string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SafeDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SafeDetails").field(&self.0).finish()
    }
}

impl fmt::Display for SafeDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Wrap a value so that its `Debug` and `Display` implementations always emit
/// `[REDACTED]`. The inner value can be accessed via `expose_secret`.
#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Wrap a sensitive value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Expose the wrapped value. This should only be used when the value is
    /// needed for cryptographic comparison or secure transport, never for logs.
    pub fn expose_secret(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_header() {
        let input = "Authorization: Digest username=alice\nX-Other: ok";
        let out = redact_details(input);
        assert_eq!(out, "Authorization: [REDACTED]\nX-Other: ok");
    }

    #[test]
    fn redacts_password_xml_element() {
        let input = "<password>supersecret</password>";
        let out = redact_details(input);
        assert_eq!(out, "[REDACTED]");
    }

    #[test]
    fn keeps_innocent_lines() {
        let input = "device_id=34020000001320000001\nstatus=online";
        let out = redact_details(input);
        assert_eq!(out, input);
    }

    #[test]
    fn redacted_display_is_masked() {
        let r = Redacted::new("secret".to_string());
        assert_eq!(format!("{r}"), "[REDACTED]");
        assert_eq!(format!("{r:?}"), "[REDACTED]");
        assert_eq!(r.expose_secret(), "secret");
    }
}

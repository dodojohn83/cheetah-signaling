//! Redacts common secret-bearing substrings from plugin log lines before they
//! are emitted into the host's structured logs.

use regex::{Captures, Regex};
use std::sync::LazyLock;

/// Returns a copy of `line` with common credentials and secret values replaced
/// by `[REDACTED]`.
///
/// Redacted patterns include:
///
/// - `Authorization: ...` HTTP/header values (rest of the line)
/// - `Bearer <token>`, `Basic <token>` and `Digest ...` authentication schemes
/// - key/value pairs where the key matches a sensitive name such as
///   `password`, `secret`, `token`, `api_key`, `client_secret`, `private_key`,
///   etc., with `=` or `:` separators and optional surrounding quotes
/// - PEM `-----BEGIN ...-----` blocks (the entire line is replaced)
pub fn sanitize_log_line(line: &str) -> String {
    if line.trim_start().starts_with("-----BEGIN") {
        return "[REDACTED PEM]".to_string();
    }

    let line = AUTH_HEADER.replace_all(line, "$1[REDACTED]");
    let line = BEARER_SCHEME.replace_all(&line, "$1[REDACTED]");
    let line = BASIC_SCHEME.replace_all(&line, "$1[REDACTED]");
    let line = DIGEST_SCHEME.replace_all(&line, "$1[REDACTED]");
    SENSITIVE_KEY_VALUE
        .replace_all(&line, |caps: &Captures<'_>| {
            format!(
                "{}{}{}{}[REDACTED]",
                &caps["open"], &caps["key"], &caps["close"], &caps["sep"]
            )
        })
        .into_owned()
}

#[allow(clippy::unwrap_used)]
static AUTH_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^.*?\bauthorization\s*:\s*)(.*)$").unwrap());

#[allow(clippy::unwrap_used)]
static BEARER_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\bbearer\s+)\S+").unwrap());

#[allow(clippy::unwrap_used)]
static BASIC_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\bbasic\s+)\S+").unwrap());

#[allow(clippy::unwrap_used)]
static DIGEST_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\bdigest\s+).*").unwrap());

#[allow(clippy::unwrap_used)]
static SENSITIVE_KEY_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?ix)
        (?P<open>["']?)
        \b(?P<key>password|passwd|pass|secret|token|api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|auth[_-]?token|client[_-]?secret|private[_-]?key|client[_-]?key)\b
        (?P<close>["']?)
        (?P<sep>\s*[:=]\s*)
        (?P<value>
            "(?:[^"\\]|\\.)*?"
            |
            '(?:[^'\\]|\\.)*?'
            |
            [^ \t\r\n;&,]+
        )"#
    )
    .unwrap()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_header() {
        let line = "request Authorization: Bearer super-secret-token extra";
        assert_eq!(sanitize_log_line(line), "request Authorization: [REDACTED]");
    }

    #[test]
    fn redacts_bearer_token() {
        assert_eq!(
            sanitize_log_line("got Bearer abc123.def"),
            "got Bearer [REDACTED]"
        );
    }

    #[test]
    fn redacts_basic_token() {
        assert_eq!(
            sanitize_log_line("header Basic dXNlcjpwYXNz"),
            "header Basic [REDACTED]"
        );
    }

    #[test]
    fn redacts_key_value_pairs() {
        assert_eq!(
            sanitize_log_line("password=secret123&api_key=xyz&token=abc"),
            "password=[REDACTED]&api_key=[REDACTED]&token=[REDACTED]"
        );
    }

    #[test]
    fn redacts_json_like_values() {
        let line = r#"{"password": "my secret", "client_secret":"abc"}"#;
        let sanitized = sanitize_log_line(line);
        assert!(!sanitized.contains("my secret"));
        assert!(!sanitized.contains("abc"));
        assert!(sanitized.contains("password"));
        assert!(sanitized.contains("client_secret"));
        assert!(sanitized.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_pem_begin_line() {
        assert_eq!(
            sanitize_log_line("-----BEGIN PRIVATE KEY-----"),
            "[REDACTED PEM]"
        );
    }

    #[test]
    fn preserves_innocent_lines() {
        let line = "connected to 192.168.1.1 port 554";
        assert_eq!(sanitize_log_line(line), line);
    }
}

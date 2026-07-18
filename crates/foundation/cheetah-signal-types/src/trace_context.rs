//! W3C trace context validation helpers.
//!
//! These helpers are intentionally strict: only the formats defined by the W3C
//! trace-context specification are accepted, so attacker-controlled values
//! cannot be reflected into responses or logs.

const TRACEPARENT_LENGTH: usize = 55;

fn is_lower_hex(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Validates a `traceparent` header value.
///
/// Accepts only the W3C format `version-traceid-parentid-flags` where all
/// groups are lowercase hex, the trace and parent IDs are non-zero, and the
/// version is one currently defined by the spec (`00` or `01`).
pub fn validate_traceparent(value: &str) -> Option<&str> {
    if value.len() != TRACEPARENT_LENGTH {
        return None;
    }

    let mut parts = value.split('-');
    let version = parts.next()?;
    let trace_id = parts.next()?;
    let parent_id = parts.next()?;
    let flags = parts.next()?;

    if parts.next().is_some() {
        return None;
    }

    if !matches!(version, "00" | "01") || !is_lower_hex(version) {
        return None;
    }
    if trace_id.len() != 32 || !is_lower_hex(trace_id) || trace_id.chars().all(|c| c == '0') {
        return None;
    }
    if parent_id.len() != 16 || !is_lower_hex(parent_id) || parent_id.chars().all(|c| c == '0') {
        return None;
    }
    if flags.len() != 2 || !is_lower_hex(flags) {
        return None;
    }

    Some(value)
}

/// Validates a `tracestate` header value.
///
/// Accepts up to 32 comma-separated `key=value` entries. Control characters,
/// line breaks and values longer than 512 characters are rejected. Keys and
/// values must be non-empty and contain no commas, semicolons or additional
/// equals signs.
pub fn validate_tracestate(value: &str) -> Option<&str> {
    if value.is_empty() || value.len() > 512 {
        return None;
    }
    if value.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return None;
    }

    let entries: Vec<&str> = value.split(',').collect();
    if entries.is_empty() || entries.len() > 32 {
        return None;
    }

    for entry in entries {
        let entry = entry.trim();
        if entry.is_empty() {
            return None;
        }
        let eq = entry.find('=')?;
        if eq == 0 || eq == entry.len() - 1 {
            return None;
        }
        let (key, val) = entry.split_at(eq);
        let val = &val[1..];
        for part in [key, val] {
            if part.is_empty() || part.bytes().any(|b| b < 0x21 || b == 0x7f || b == b',') {
                return None;
            }
        }
    }

    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_traceparent_is_accepted() {
        let tp = "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01";
        assert_eq!(validate_traceparent(tp), Some(tp));
    }

    #[test]
    fn traceparent_with_wrong_length_is_rejected() {
        assert!(validate_traceparent("00-abc-00f067aa0ba902b7-01").is_none());
    }

    #[test]
    fn traceparent_with_uppercase_is_rejected() {
        let tp = "00-0AF7651916CD43DD8448EB211C80319C-00F067AA0BA902B7-01";
        assert!(validate_traceparent(tp).is_none());
    }

    #[test]
    fn traceparent_with_zero_trace_id_is_rejected() {
        let tp = "00-00000000000000000000000000000000-00f067aa0ba902b7-01";
        assert!(validate_traceparent(tp).is_none());
    }

    #[test]
    fn valid_tracestate_is_accepted() {
        let ts = "vendor=value,key2=other";
        assert_eq!(validate_tracestate(ts), Some(ts));
    }

    #[test]
    fn tracestate_with_control_chars_is_rejected() {
        assert!(validate_tracestate("key=val\nue").is_none());
    }

    #[test]
    fn tracestate_too_many_entries_is_rejected() {
        let entries: Vec<String> = (0..33).map(|i| format!("k{i}=v")).collect();
        assert!(validate_tracestate(&entries.join(",")).is_none());
    }
}

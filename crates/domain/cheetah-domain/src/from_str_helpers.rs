//! Small helpers for case-insensitive, allocation-free `FromStr` implementations.

/// Maximum characters from an untrusted string to include in an error message.
pub(crate) const MAX_FROM_STR_DISPLAY_LEN: usize = 64;

/// Truncates `s` to [`MAX_FROM_STR_DISPLAY_LEN`] characters for safe error diagnostics.
pub(crate) fn truncate_for_error(s: &str) -> String {
    s.chars().take(MAX_FROM_STR_DISPLAY_LEN).collect()
}

/// Case-insensitive equality where hyphens in `s` are treated as underscores.
///
/// `target` must already be normalized lower-case snake_case. This avoids
/// allocating a normalized copy of `s` and is bounded by the length of `s`.
pub(crate) fn eq_normalized_snake(s: &str, target: &str) -> bool {
    let mut target_iter = target.as_bytes().iter().copied();
    for c in s.as_bytes().iter().copied() {
        let expected = if c == b'-' {
            b'_'
        } else {
            c.to_ascii_lowercase()
        };
        if target_iter.next() != Some(expected) {
            return false;
        }
    }
    target_iter.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_normalized_snake_matches_case_and_hyphen_variants() {
        assert!(eq_normalized_snake("media-node", "media_node"));
        assert!(eq_normalized_snake("Media_Node", "media_node"));
        assert!(eq_normalized_snake("MEDIA-NODE", "media_node"));
        assert!(eq_normalized_snake("signaling_host", "signaling_host"));
    }

    #[test]
    fn eq_normalized_snake_rejects_mismatches() {
        assert!(!eq_normalized_snake("media_node", "signaling_host"));
        assert!(!eq_normalized_snake("media-node", "media_node_extra"));
        assert!(!eq_normalized_snake("media--node", "media_node"));
    }

    #[test]
    fn truncate_for_error_bounds_output() {
        let long = "x".repeat(128);
        let truncated = truncate_for_error(&long);
        assert_eq!(truncated.len(), MAX_FROM_STR_DISPLAY_LEN);
    }
}

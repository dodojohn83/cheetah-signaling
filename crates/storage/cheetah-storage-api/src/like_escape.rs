//! Shared SQL `LIKE` pattern escaping helpers for storage adapters.

/// Escapes `%`, `_` and `\` in `value` so it is treated as a literal prefix
/// when used with `LIKE ... ESCAPE '\'`.
pub fn escape_like_pattern(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c == '\\' || c == '%' || c == '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_like_pattern_escapes_metacharacters() {
        assert_eq!(escape_like_pattern("a%b_c\\d"), "a\\%b\\_c\\\\d");
    }

    #[test]
    fn escape_like_pattern_is_identity_for_plain_text() {
        assert_eq!(escape_like_pattern("alice"), "alice");
    }
}

//! Revision conversion helpers shared by storage backends.

use cheetah_domain::DomainError;

/// Converts a revision stored as a signed database integer to `u64`.
///
/// Negative values indicate a corrupt or inconsistent database row and are
/// surfaced as an internal error rather than silently clamped to zero.
pub fn stored_revision_as_u64(r: i64) -> Result<u64, DomainError> {
    u64::try_from(r)
        .map_err(|_| DomainError::internal(format!("stored revision is out of range: {r}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_revision_as_u64_accepts_zero_and_positive() {
        assert!(matches!(stored_revision_as_u64(0), Ok(0)));
        assert!(matches!(stored_revision_as_u64(42), Ok(42)));
    }

    #[test]
    fn stored_revision_as_u64_rejects_negative() {
        assert!(matches!(
            stored_revision_as_u64(-1),
            Err(DomainError::Internal { .. })
        ));
    }
}

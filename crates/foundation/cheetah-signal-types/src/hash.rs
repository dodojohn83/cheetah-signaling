//! Stable, portable hashing utilities.

use std::hash::{Hash, Hasher};

/// Returns a 64-bit deterministic hash for `value` using the FNV-1a hasher.
///
/// Unlike `std::collections::hash_map::DefaultHasher`, this hash is stable
/// across Rust toolchain upgrades and platforms, making it suitable for
/// connection-pool keys, shard routing and deterministic jitter.
pub fn stable_hash_u64<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = fnv::FnvHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn stable_hash_is_deterministic_for_strings() {
        let a = stable_hash_u64("hello");
        let b = stable_hash_u64("hello");
        assert_eq!(a, b);
        assert_ne!(a, stable_hash_u64("world"));
    }

    #[test]
    fn stable_hash_distinguishes_options() {
        assert_ne!(stable_hash_u64(&Some("x")), stable_hash_u64(&None::<&str>));
    }
}

//! Deterministic RNG stream derivation.
//!
//! Every stochastic decision in the simulator draws from a named, independent
//! RNG stream derived from the master seed.  Deriving per-stream seeds via a
//! stable hash keeps decisions reproducible and prevents one subsystem's draws
//! from shifting another's when a scenario changes.

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use sha2::{Digest, Sha256};

/// Derives a child seed from a master seed and a stable string label.
pub fn derive_seed(master: u64, label: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(master.to_le_bytes());
    hasher.update([0u8]);
    hasher.update(label.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(bytes)
}

/// Builds a fresh RNG for the named stream derived from `master`.
pub fn stream_rng(master: u64, label: &str) -> StdRng {
    StdRng::seed_from_u64(derive_seed(master, label))
}

/// Builds a fresh RNG for the named indexed stream (e.g. per shard/device).
pub fn indexed_rng(master: u64, label: &str, index: u64) -> StdRng {
    StdRng::seed_from_u64(derive_seed(master, &format!("{label}:{index}")))
}

/// Returns `true` with probability `rate`, drawing from `rng`.
pub fn bernoulli(rng: &mut StdRng, rate: f64) -> bool {
    if rate <= 0.0 {
        return false;
    }
    if rate >= 1.0 {
        return true;
    }
    rng.r#gen::<f64>() < rate
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn derive_seed_is_stable_and_label_sensitive() {
        assert_eq!(derive_seed(42, "faults"), derive_seed(42, "faults"));
        assert_ne!(derive_seed(42, "faults"), derive_seed(42, "clock"));
        assert_ne!(derive_seed(42, "faults"), derive_seed(43, "faults"));
    }

    #[test]
    fn bernoulli_bounds() {
        let mut rng = stream_rng(1, "t");
        assert!(!bernoulli(&mut rng, 0.0));
        assert!(bernoulli(&mut rng, 1.0));
    }

    #[test]
    fn stream_rng_is_reproducible() {
        let mut a = stream_rng(7, "x");
        let mut b = stream_rng(7, "x");
        let va: u64 = a.r#gen();
        let vb: u64 = b.r#gen();
        assert_eq!(va, vb);
    }
}

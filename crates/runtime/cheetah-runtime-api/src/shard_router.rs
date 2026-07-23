//! Stable hash routing from a device key to a fixed shard index.

use cheetah_signal_types::hash::stable_hash_u64;

use crate::DeviceKey;

/// Routes device keys to shard indices using a stable hash.
#[derive(Clone, Debug)]
pub struct ShardRouter {
    shard_count: usize,
}

impl ShardRouter {
    /// Creates a new router with the given shard count.
    pub fn new(shard_count: usize) -> Self {
        Self { shard_count }
    }

    /// Returns the shard index for a device key.
    pub fn route(&self, key: DeviceKey) -> usize {
        if self.shard_count == 0 {
            return 0;
        }
        (stable_hash_u64(&key) as usize) % self.shard_count
    }

    /// Returns the configured shard count.
    pub fn shard_count(&self) -> usize {
        self.shard_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    use cheetah_signal_types::{DeviceId, TenantId};

    #[test]
    fn route_is_stable_for_same_key() {
        let router = ShardRouter::new(16);
        let tenant =
            TenantId::from_uuid(Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111));
        let device =
            DeviceId::from_uuid(Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222));
        let key = DeviceKey::new(tenant, device);
        let idx1 = router.route(key);
        let idx2 = router.route(key);
        assert!(idx1 < 16);
        assert_eq!(idx1, idx2);
    }

    #[test]
    fn different_devices_can_land_on_different_shards() {
        let router = ShardRouter::new(16);
        let tenant =
            TenantId::from_uuid(Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111));
        let device1 =
            DeviceId::from_uuid(Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222));
        let device2 =
            DeviceId::from_uuid(Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333));
        let key1 = DeviceKey::new(tenant, device1);
        let key2 = DeviceKey::new(tenant, device2);
        let idx1 = router.route(key1);
        let idx2 = router.route(key2);
        // The probability of a collision is low but not zero; this test mainly
        // checks determinism and range.
        assert!(idx1 < 16);
        assert!(idx2 < 16);
        assert_eq!(router.route(key1), idx1);
        assert_eq!(router.route(key2), idx2);
    }

    #[test]
    fn zero_shard_count_falls_back_to_index_zero() {
        let router = ShardRouter::new(0);
        let key = DeviceKey::new(TenantId::generate(), DeviceId::generate());
        assert_eq!(router.route(key), 0);
    }
}

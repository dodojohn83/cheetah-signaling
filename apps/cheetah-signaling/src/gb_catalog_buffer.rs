//! In-memory accumulator for paginated GB28181 catalog fragments.

use cheetah_gb28181_module::{DeviceId as GbDeviceId, xml::CatalogItem as GbCatalogItem};
use cheetah_signal_types::TenantId;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::warn;

/// Creates a stable key for a catalog fragment from the sorted set of channel
/// external ids it contains. Retransmissions of the same fragment will share
/// the same key and therefore contribute their declared `Num` only once.
pub(crate) fn fragment_key(batch: &HashMap<String, GbCatalogItem>) -> String {
    if batch.is_empty() {
        return "empty".to_string();
    }
    let mut ids: Vec<_> = batch.keys().map(String::as_str).collect();
    ids.sort();
    ids.join(",")
}

pub(crate) const CATALOG_FRAGMENT_TTL: Duration = Duration::from_secs(60);
pub(crate) const CATALOG_CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

/// In-memory accumulator for paginated GB28181 catalog fragments.
///
/// Devices may split a catalog response across multiple SIP MESSAGE bodies.
/// Fragments are keyed by (tenant, device, sequence number) and merged into a
/// single `replace_channel_catalog` call once `sum_num` distinct channel ids
/// have been seen. Items are de-duplicated by their channel `device_id`.
///
/// To avoid stalling when a camera drops malformed items, completion also falls
/// back to the sum of declared `Num` values for each *unique* fragment content
/// (retransmissions are ignored). If the unique fragment count equals `sum_num`
/// but fewer distinct channels were collected, the partial catalog is emitted as
/// a best-effort replacement and a warning is logged. Overlapping fragments that
/// would push the unique declared count above `sum_num` are not used to trigger
/// completion. Partial transfers expire after `CATALOG_FRAGMENT_TTL` and are
/// evicted by the background worker cleanup tick.
pub(crate) struct CatalogBuffer {
    entries: HashMap<CatalogKey, PartialCatalog>,
    max_entries: usize,
    max_items_per_entry: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct CatalogKey {
    tenant_id: TenantId,
    device_id: String,
    sn: String,
}

pub(crate) struct PartialCatalog {
    /// Accumulated catalog items keyed by channel external id (`device_id`).
    items: HashMap<String, GbCatalogItem>,
    /// Declared total number of items across all fragments (`SumNum`).
    expected: u32,
    /// Declared `Num` values keyed by a digest of the fragment's channel ids.
    ///
    /// Retransmissions of the same fragment share the same key and do not
    /// contribute multiple times. For each unique fragment the largest `Num`
    /// value observed is kept.
    fragments: HashMap<String, u32>,
    last_seen: Instant,
}

impl PartialCatalog {
    /// Sum of declared `Num` values for unique fragments received so far.
    fn received_num(&self) -> u32 {
        self.fragments
            .values()
            .copied()
            .fold(0u32, |acc, v| acc.saturating_add(v))
    }

    fn is_complete(&self) -> bool {
        let distinct = self.items.len();
        if distinct >= self.expected as usize {
            if distinct > self.expected as usize {
                warn!(
                    expected = self.expected,
                    distinct, "gb28181 catalog has more distinct channels than declared"
                );
            }
            return true;
        }

        let received = self.received_num();
        if received == self.expected {
            warn!(
                expected = self.expected,
                distinct,
                received,
                "gb28181 catalog unique fragment count reached sum_num with fewer distinct channels; some items may have been malformed or dropped"
            );
            return true;
        }

        if received > self.expected {
            warn!(
                expected = self.expected,
                distinct,
                received,
                "gb28181 catalog fragments overlap or repeat declared counts; waiting for distinct channel ids"
            );
        }

        false
    }
}

impl CatalogBuffer {
    pub(crate) fn new(max_entries: usize, max_items_per_entry: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            max_items_per_entry,
        }
    }

    pub(crate) fn accumulate(
        &mut self,
        tenant_id: TenantId,
        device_id: &GbDeviceId,
        sn: &str,
        expected: u32,
        num: u32,
        items: Vec<GbCatalogItem>,
    ) -> Option<Vec<GbCatalogItem>> {
        // De-duplicate within the incoming fragment before any size checks.
        let mut batch = HashMap::with_capacity(items.len());
        for item in items {
            batch.insert(item.device_id.clone(), item);
        }

        if expected == 0 {
            return Some(batch.into_values().collect());
        }

        let expected_usize = expected as usize;
        if expected_usize > self.max_items_per_entry {
            warn!(
                %device_id,
                sn,
                expected,
                max_items_per_entry = self.max_items_per_entry,
                "gb28181 catalog fragment declares more items than allowed; dropping"
            );
            return None;
        }

        let key = CatalogKey {
            tenant_id,
            device_id: device_id.as_ref().to_string(),
            sn: sn.to_string(),
        };

        if !self.entries.contains_key(&key) && self.entries.len() >= self.max_entries {
            warn!(
                sn,
                max_entries = self.max_entries,
                "gb28181 catalog fragment buffer full; dropping new fragment"
            );
            return None;
        }

        if let Some(partial) = self.entries.get_mut(&key) {
            let new_distinct = batch
                .keys()
                .filter(|k| !partial.items.contains_key(*k))
                .count();
            let total = partial.items.len().saturating_add(new_distinct);
            if total > self.max_items_per_entry {
                warn!(
                    %device_id,
                    sn,
                    accumulated = total,
                    max_items_per_entry = self.max_items_per_entry,
                    "gb28181 catalog fragment exceeded per-entry item limit; dropping partial"
                );
                self.entries.remove(&key);
                return None;
            }
            partial
                .fragments
                .entry(fragment_key(&batch))
                .and_modify(|v| *v = num.max(*v))
                .or_insert(num);
            partial.items.extend(batch);
            partial.last_seen = Instant::now();
            if partial.is_complete() {
                return self
                    .entries
                    .remove(&key)
                    .map(|complete| complete.items.into_values().collect());
            }
            return None;
        }

        if batch.len() > self.max_items_per_entry {
            warn!(
                %device_id,
                sn,
                accumulated = batch.len(),
                max_items_per_entry = self.max_items_per_entry,
                "gb28181 catalog fragment exceeded per-entry item limit; dropping"
            );
            return None;
        }

        let mut fragments = HashMap::new();
        if num > 0 || !batch.is_empty() {
            fragments.insert(fragment_key(&batch), num);
        }
        let partial = PartialCatalog {
            items: batch,
            expected,
            fragments,
            last_seen: Instant::now(),
        };
        if partial.is_complete() {
            return Some(partial.items.into_values().collect());
        }
        self.entries.insert(key, partial);
        None
    }

    pub(crate) fn evict(&mut self) {
        let now = Instant::now();
        let before = self.entries.len();
        self.entries
            .retain(|_, partial| now.duration_since(partial.last_seen) <= CATALOG_FRAGMENT_TTL);
        let dropped = before.saturating_sub(self.entries.len());
        if dropped > 0 {
            warn!(
                dropped,
                "gb28181 catalog fragment buffer evicted stale entries"
            );
        }
    }
}

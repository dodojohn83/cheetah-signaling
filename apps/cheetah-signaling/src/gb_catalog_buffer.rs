//! In-memory accumulator for paginated GB28181 catalog and record-info fragments.

use cheetah_gb28181_module::{
    DeviceId as GbDeviceId,
    xml::{CatalogItem as GbCatalogItem, RecordItem},
};
use cheetah_signal_types::TenantId;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::warn;

const TTL: Duration = Duration::from_secs(60);
pub(crate) const CATALOG_CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

/// Items that can be accumulated across paginated SIP MESSAGE fragments.
pub(crate) trait FragmentItem: Clone {
    /// Human-readable label used in diagnostic logs.
    const LABEL: &'static str;

    /// Stable key that uniquely identifies this item within a single fragment
    /// and across fragments. Retransmissions of the same fragment will produce
    /// the same set of keys and therefore contribute their declared `Num` only
    /// once.
    fn stable_key(&self) -> String;
}

impl FragmentItem for GbCatalogItem {
    const LABEL: &'static str = "catalog";

    fn stable_key(&self) -> String {
        self.device_id.clone()
    }
}

impl FragmentItem for RecordItem {
    const LABEL: &'static str = "record";

    fn stable_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.device_id,
            self.start_time.as_deref().unwrap_or(""),
            self.end_time.as_deref().unwrap_or(""),
            self.file_path.as_deref().unwrap_or("")
        )
    }
}

/// Creates a stable key for a fragment from the sorted set of item keys it
/// contains. Retransmissions of the same fragment will share the same key and
/// therefore contribute their declared `Num` only once.
fn fragment_key<T>(batch: &HashMap<String, T>) -> String {
    if batch.is_empty() {
        return "empty".to_string();
    }
    let mut ids: Vec<_> = batch.keys().map(String::as_str).collect();
    ids.sort();
    ids.join(",")
}

/// In-memory accumulator for paginated GB28181 fragment responses.
///
/// Devices may split a catalog or record-info response across multiple SIP
/// MESSAGE bodies. Fragments are keyed by (tenant, device, sequence number)
/// and merged into a single collection once `sum_num` distinct item keys have
/// been seen. Items are de-duplicated by their [`FragmentItem::stable_key`].
///
/// To avoid stalling when a camera drops malformed items, completion also falls
/// back to the sum of declared `Num` values for each *unique* fragment content
/// (retransmissions are ignored). If the unique fragment count equals `sum_num`
/// but fewer distinct items were collected, the partial result is emitted as a
/// best-effort replacement and a warning is logged. Overlapping fragments that
/// would push the unique declared count above `sum_num` are not used to trigger
/// completion. Partial transfers expire after 60 seconds and are
/// evicted by the background worker cleanup tick.
pub(crate) struct FragmentBuffer<T: FragmentItem> {
    entries: HashMap<FragmentKey, PartialFragment<T>>,
    max_entries: usize,
    max_items_per_entry: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct FragmentKey {
    tenant_id: TenantId,
    device_id: String,
    sn: String,
}

struct PartialFragment<T: FragmentItem> {
    /// Accumulated items keyed by [`FragmentItem::stable_key`].
    items: HashMap<String, T>,
    /// Declared total number of items across all fragments (`SumNum`).
    expected: u32,
    /// Declared `Num` values keyed by a digest of the fragment's item keys.
    ///
    /// Retransmissions of the same fragment share the same key and do not
    /// contribute multiple times. For each unique fragment the largest `Num`
    /// value observed is kept.
    fragments: HashMap<String, u32>,
    last_seen: Instant,
}

impl<T: FragmentItem> PartialFragment<T> {
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
                    distinct,
                    "gb28181 {} has more distinct items than declared",
                    T::LABEL,
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
                "gb28181 {} unique fragment count reached sum_num with fewer distinct items; some items may have been malformed or dropped",
                T::LABEL,
            );
            return true;
        }

        if received > self.expected {
            warn!(
                expected = self.expected,
                distinct,
                received,
                "gb28181 {} fragments overlap or repeat declared counts; waiting for distinct item ids",
                T::LABEL,
            );
        }

        false
    }
}

impl<T: FragmentItem> FragmentBuffer<T> {
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
        items: Vec<T>,
    ) -> Option<Vec<T>> {
        // De-duplicate within the incoming fragment before any size checks.
        let mut batch = HashMap::with_capacity(items.len());
        for item in items {
            batch.insert(item.stable_key(), item);
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
                "gb28181 {} fragment declares more items than allowed; dropping",
                T::LABEL,
            );
            return None;
        }

        let key = FragmentKey {
            tenant_id,
            device_id: device_id.as_ref().to_string(),
            sn: sn.to_string(),
        };

        if !self.entries.contains_key(&key) && self.entries.len() >= self.max_entries {
            warn!(
                sn,
                max_entries = self.max_entries,
                "gb28181 {} fragment buffer full; dropping new fragment",
                T::LABEL,
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
                    "gb28181 {} fragment exceeded per-entry item limit; dropping partial",
                    T::LABEL,
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
                "gb28181 {} fragment exceeded per-entry item limit; dropping",
                T::LABEL,
            );
            return None;
        }

        let mut fragments = HashMap::new();
        if num > 0 || !batch.is_empty() {
            fragments.insert(fragment_key(&batch), num);
        }
        let partial = PartialFragment {
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
            .retain(|_, partial| now.duration_since(partial.last_seen) <= TTL);
        let dropped = before.saturating_sub(self.entries.len());
        if dropped > 0 {
            warn!(
                dropped,
                "gb28181 {} fragment buffer evicted stale entries",
                T::LABEL,
            );
        }
    }
}

pub(crate) type CatalogBuffer = FragmentBuffer<GbCatalogItem>;
pub(crate) type RecordInfoBuffer = FragmentBuffer<RecordItem>;

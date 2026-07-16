//! Catalog aggregation for the GB28181 state machine.

use crate::output::{Gb28181Catalog, Gb28181CatalogItem};
use crate::xml::{Gb28181Message, Item};

use super::CatalogAggregator;
use super::Gb28181Module;

impl Gb28181Module {
    /// Aggregates catalog MESSAGE slices into a single output once all fragments
    /// have been received or the configured page size cap is reached.
    pub(super) fn aggregate_catalog(&mut self, msg: &Gb28181Message) -> Option<Gb28181Catalog> {
        let sn = msg.sn.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
        let declared_sum = msg.sum_num.unwrap_or(0);
        let items: Vec<Gb28181CatalogItem> = msg
            .item_list
            .as_ref()
            .map(|list| list.item.iter().map(catalog_item_from_dto).collect())
            .unwrap_or_default();

        let page_size = self.config.catalog_page_size;
        let sum_num = declared_sum.max(items.len() as u32);
        let target = sum_num.min(page_size);
        let capped = sum_num > page_size;

        if let Some(agg) = self.catalog.as_mut()
            && agg.sn == sn
        {
            agg.items.extend(items);
            agg.received_fragments += 1;
            agg.sum_num = agg.sum_num.max(sum_num);
            agg.target = agg.target.max(target);
            agg.capped = agg.capped || capped;
            if agg.items.len() as u32 >= agg.target {
                return self.take_catalog(sn, msg);
            }
            return None;
        }

        if items.len() as u32 >= target {
            return Some(Gb28181Catalog {
                device_id: msg.device_id.clone().unwrap_or_default(),
                sn,
                sum_num,
                items,
                complete: !capped,
            });
        }

        self.catalog = Some(CatalogAggregator {
            sn,
            sum_num,
            target,
            capped,
            items,
            received_fragments: 1,
        });
        None
    }

    fn take_catalog(&mut self, sn: u32, msg: &Gb28181Message) -> Option<Gb28181Catalog> {
        let agg = self.catalog.take()?;
        Some(Gb28181Catalog {
            device_id: msg.device_id.clone().unwrap_or_default(),
            sn,
            sum_num: agg.sum_num,
            items: agg.items,
            complete: !agg.capped,
        })
    }
}

fn catalog_item_from_dto(item: &Item) -> Gb28181CatalogItem {
    Gb28181CatalogItem {
        device_id: item.device_id.clone().unwrap_or_default(),
        name: item.name.clone(),
        status: item.status.clone(),
        parental: item.parental.as_deref().and_then(|s| s.parse().ok()),
        parent_id: item.parent_id.clone(),
        longitude: item.longitude.as_deref().and_then(|s| s.parse().ok()),
        latitude: item.latitude.as_deref().and_then(|s| s.parse().ok()),
        manufacturer: item.manufacturer.clone(),
        model: item.model.clone(),
        ip_address: item.ip_address.clone(),
        port: item.port,
    }
}

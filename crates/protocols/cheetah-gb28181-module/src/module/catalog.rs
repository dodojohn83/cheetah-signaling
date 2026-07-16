//! Catalog aggregation for the GB28181 state machine.

use crate::output::{Gb28181Catalog, Gb28181CatalogItem};
use crate::xml::{Gb28181Message, Item};

use super::Gb28181Module;

impl Gb28181Module {
    /// Aggregates catalog MESSAGE slices into a single output once all fragments
    /// have been received.
    pub(super) fn aggregate_catalog(&mut self, msg: &Gb28181Message) -> Option<Gb28181Catalog> {
        let sn = msg.sn.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
        let sum_num = msg.sum_num.unwrap_or(0);
        let items: Vec<Gb28181CatalogItem> = msg
            .item_list
            .as_ref()
            .map(|list| list.item.iter().map(catalog_item_from_dto).collect())
            .unwrap_or_default();

        if let Some(agg) = self.catalog.as_mut()
            && agg.sn == sn
        {
            agg.items.extend(items);
            agg.received_fragments += 1;
            if agg.items.len() as u32 >= agg.sum_num {
                let catalog = Gb28181Catalog {
                    device_id: msg.device_id.clone().unwrap_or_default(),
                    sn,
                    sum_num: agg.sum_num,
                    items: agg.items.clone(),
                    complete: true,
                };
                self.catalog = None;
                return Some(catalog);
            }
            return None;
        }

        if items.len() as u32 >= sum_num {
            return Some(Gb28181Catalog {
                device_id: msg.device_id.clone().unwrap_or_default(),
                sn,
                sum_num,
                items,
                complete: true,
            });
        }

        self.catalog = Some(super::CatalogAggregator {
            sn,
            sum_num,
            items,
            received_fragments: 1,
        });
        None
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

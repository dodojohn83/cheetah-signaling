//! Channel-catalog replacement and channel metadata construction.

use cheetah_gb28181_module::DeviceId as GbDeviceId;
use cheetah_gb28181_module::xml::CatalogItem as GbCatalogItem;
use cheetah_http_api::state::ApiState;
use cheetah_signal_application::{ChannelDescriptor, ReplaceChannelCatalogRequest};
use cheetah_signal_types::{RequestContext, SignalError, TenantId};
use std::collections::BTreeMap;

use super::device::resolve_device_id;
use super::storage_error;

pub(super) async fn replace_catalog(
    state: &ApiState,
    context: &RequestContext,
    tenant_id: TenantId,
    device_id: &GbDeviceId,
    items: &[GbCatalogItem],
) -> Result<(), SignalError> {
    let external_id = device_id.as_ref();
    let internal_id = match resolve_device_id(state, tenant_id, external_id).await {
        Some(id) => id,
        None => return Ok(()),
    };

    let mut channels = Vec::with_capacity(items.len());
    for item in items {
        let channel_id = cheetah_domain::channel::map_gb28181_channel_id(
            tenant_id,
            external_id,
            &item.device_id,
        );
        channels.push(ChannelDescriptor {
            id: Some(channel_id.to_string()),
            name: item.name.clone().unwrap_or_else(|| item.device_id.clone()),
            kind: "video".to_string(),
            enabled: true,
            status: None,
            stream_profiles: vec![],
            ptz_capabilities: None,
            metadata: Some(build_channel_metadata(item)),
        });
    }

    let mut uow = state.storage.begin().await.map_err(storage_error)?;
    let _ = state
        .device_service
        .replace_channel_catalog(
            context,
            &mut *uow,
            internal_id,
            ReplaceChannelCatalogRequest { channels },
        )
        .await?;
    Ok(())
}

/// Builds channel metadata from a GB28181 catalog item.
///
/// The channel's GB28181 external id (`item.device_id`) is stored under the
/// `external_id` key so that channel-scoped commands (PTZ, preset, etc.) can
/// recover the channel's protocol identifier: the internal `ChannelId` is a
/// one-way UUIDv5 hash and cannot be reversed to the original id.
fn build_channel_metadata(item: &GbCatalogItem) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    metadata.insert("external_id".to_string(), item.device_id.clone());
    if let Some(v) = &item.manufacturer {
        metadata.insert("manufacturer".to_string(), v.clone());
    }
    if let Some(v) = &item.model {
        metadata.insert("model".to_string(), v.clone());
    }
    if let Some(v) = &item.owner {
        metadata.insert("owner".to_string(), v.clone());
    }
    if let Some(v) = &item.civil_code {
        metadata.insert("civil_code".to_string(), v.clone());
    }
    if let Some(v) = &item.block {
        metadata.insert("block".to_string(), v.clone());
    }
    if let Some(v) = &item.address {
        metadata.insert("address".to_string(), v.clone());
    }
    if let Some(v) = &item.parent_id {
        metadata.insert("parent_id".to_string(), v.clone());
    }
    if let Some(v) = &item.ip_address {
        metadata.insert("ip_address".to_string(), v.clone());
    }
    if let Some(v) = &item.port {
        metadata.insert("port".to_string(), v.clone());
    }
    if let Some(v) = &item.status {
        metadata.insert("status".to_string(), v.clone());
    }
    if let Some(v) = &item.longitude {
        metadata.insert("longitude".to_string(), v.clone());
    }
    if let Some(v) = &item.latitude {
        metadata.insert("latitude".to_string(), v.clone());
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_metadata_records_external_id() {
        let item = GbCatalogItem {
            device_id: "34020000001320000001".to_string(),
            manufacturer: Some("Acme".to_string()),
            ..Default::default()
        };

        let metadata = build_channel_metadata(&item);

        assert_eq!(
            metadata.get("external_id").map(String::as_str),
            Some("34020000001320000001"),
            "channel external id must be persisted so channel-scoped commands resolve the right target"
        );
        assert_eq!(
            metadata.get("manufacturer").map(String::as_str),
            Some("Acme")
        );
    }
}

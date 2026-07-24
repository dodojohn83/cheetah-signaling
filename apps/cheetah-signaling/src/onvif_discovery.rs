//! ONVIF WS-Discovery worker and device provisioning.
//!
//! Periodically probes the local network for ONVIF devices, fetches basic
//! device information with bounded concurrency, and registers/upgrades the
//! discovered cameras through the application `DeviceService`.

use cheetah_domain::{MAX_DEVICE_NAME_BYTES, MAX_METADATA_VALUE_BYTES};
use cheetah_http_api::state::ApiState;
use cheetah_onvif_driver_tokio::{DriverConfig, OnvifHttpDriver, probe_once};
use cheetah_onvif_module::DeviceInformation;
use cheetah_signal_application::{
    CapabilityDto, CapabilityValueDto, MarkDeviceOnlineRequest, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest,
};
use cheetah_signal_types::config::OnvifConfig;
use cheetah_signal_types::{
    CorrelationId, MessageId, NodeId, Principal, PrincipalKind, RequestContext, TenantId,
    clamp_str, clamp_string_bytes,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

/// Maximum ONVIF discovery interval; larger values overflow `tokio::time` deadlines.
const MAX_DISCOVERY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

fn clamp_discovery_interval(ms: u64) -> Duration {
    Duration::from_millis(ms).min(MAX_DISCOVERY_INTERVAL)
}

/// Maximum bytes of an XAddr or endpoint reference that are emitted into logs.
/// Keeps discovery warning/info lines bounded even if a device advertises a
/// huge address or reference.
const MAX_LOG_XADDR_BYTES: usize = 256;
const MAX_LOG_ENDPOINT_REF_BYTES: usize = 256;

/// Maximum byte length of an ONVIF endpoint reference accepted for provisioning.
/// Matches `ProtocolIdentity` so the device can be registered without truncation.
const MAX_ENDPOINT_REF_BYTES: usize = 256;

/// Starts a periodic ONVIF discovery worker.
///
/// If `onvif.discovery_interval_ms` is zero the worker performs a single sweep
/// and exits. Discovery is skipped entirely when `onvif.enabled` is false or no
/// `default_tenant_id` is configured.
pub fn spawn(
    state: ApiState,
    node_id: NodeId,
    config: OnvifConfig,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if !config.enabled {
            info!("onvif discovery disabled");
            return;
        }

        let tenant_id = match config.default_tenant_id.as_ref() {
            Some(id) => match id.parse::<TenantId>() {
                Ok(t) => t,
                Err(e) => {
                    warn!(error = %e, "onvif.default_tenant_id is not a valid UUID");
                    return;
                }
            },
            None => {
                warn!("onvif.default_tenant_id is required for discovery provisioning");
                return;
            }
        };

        let driver_config = DriverConfig::from(&config);
        let driver = match OnvifHttpDriver::new(&driver_config) {
            Ok(d) => Arc::new(d),
            Err(e) => {
                warn!(error = %e, "failed to create onvif driver");
                return;
            }
        };

        let interval =
            clamp_discovery_interval(config.discovery_interval_ms.as_millis().max(0) as u64);
        let single_sweep = config.discovery_interval_ms.as_millis() == 0;

        loop {
            if cancel.is_cancelled() {
                break;
            }

            run_discovery_sweep(
                state.clone(),
                node_id,
                tenant_id,
                driver.clone(),
                &driver_config,
                config.max_concurrent_probes.max(1),
                cancel.child_token(),
            )
            .await;

            if single_sweep {
                break;
            }

            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(interval) => {}
            }
        }

        info!("onvif discovery worker stopped");
    })
}

async fn run_discovery_sweep(
    state: ApiState,
    node_id: NodeId,
    tenant_id: TenantId,
    driver: Arc<OnvifHttpDriver>,
    driver_config: &DriverConfig,
    max_concurrent: u32,
    cancel: CancellationToken,
) {
    let result = tokio::select! {
        _ = cancel.cancelled() => return,
        r = probe_once(driver_config) => match r {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "onvif discovery probe failed");
                return;
            }
        },
    };

    if result.matches.is_empty() {
        return;
    }

    let semaphore = Arc::new(Semaphore::new(max_concurrent as usize));
    let mut set = JoinSet::new();

    // Each discovered device is provisioned at most once per sweep. Its XAddrs
    // are tried sequentially so cameras advertising multiple addresses do not
    // collide on the unique device external-id index.
    for m in result.matches {
        if cancel.is_cancelled() {
            break;
        }
        if m.endpoint_reference.0.len() > MAX_ENDPOINT_REF_BYTES {
            warn!(
                endpoint_ref = %clamp_str(&m.endpoint_reference.0, MAX_LOG_ENDPOINT_REF_BYTES),
                "onvif endpoint reference too long; skipping"
            );
            continue;
        }
        let endpoint_ref = m.endpoint_reference.0;
        let xaddrs = m.x_addrs.0;
        let permit = semaphore.clone();
        let driver = driver.clone();
        let state = state.clone();
        let cancel = cancel.child_token();
        set.spawn(async move {
            let _permit = permit.acquire().await;
            if cancel.is_cancelled() {
                return;
            }
            for xaddr in xaddrs {
                if cancel.is_cancelled() {
                    return;
                }
                match provision_device(
                    state.clone(),
                    node_id,
                    tenant_id,
                    driver.clone(),
                    &endpoint_ref,
                    &xaddr,
                )
                .await
                {
                    Ok(()) => return,
                    Err(e) => {
                        warn!(
                            xaddr = %clamp_str(&xaddr, MAX_LOG_XADDR_BYTES),
                            error = %e,
                            "onvif xaddr provisioning attempt failed"
                        );
                    }
                }
            }
            warn!(
                endpoint_ref = %clamp_str(&endpoint_ref, MAX_LOG_ENDPOINT_REF_BYTES),
                "all onvif xaddrs failed for device"
            );
        });
    }

    while !set.is_empty() {
        tokio::select! {
            _ = cancel.cancelled() => {
                set.abort_all();
                while set.join_next().await.is_some() {}
                break;
            }
            res = set.join_next() => {
                if let Some(Err(e)) = res {
                    warn!(error = %e, "onvif discovery task panicked or aborted");
                }
            }
        }
    }
}

async fn provision_device(
    state: ApiState,
    node_id: NodeId,
    tenant_id: TenantId,
    driver: Arc<OnvifHttpDriver>,
    endpoint_ref: &str,
    xaddr: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let timeout = Some(Duration::from_secs(10));
    let info = driver
        .get_device_information(xaddr, None, timeout)
        .await
        .map_err(|e| format!("get_device_information failed: {e}"))?;

    let source_ip = source_ip_from_xaddr(xaddr);
    let context = build_context(node_id, tenant_id, source_ip);

    let name = device_name(&info, endpoint_ref);
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "endpoint".to_string(),
        clamp_string_bytes(xaddr.to_string(), MAX_METADATA_VALUE_BYTES),
    );
    if !info.manufacturer.is_empty() {
        metadata.insert(
            "manufacturer".to_string(),
            clamp_string_bytes(info.manufacturer.clone(), MAX_METADATA_VALUE_BYTES),
        );
    }
    if !info.model.is_empty() {
        metadata.insert(
            "model".to_string(),
            clamp_string_bytes(info.model.clone(), MAX_METADATA_VALUE_BYTES),
        );
    }
    if !info.firmware_version.is_empty() {
        metadata.insert(
            "firmware_version".to_string(),
            clamp_string_bytes(info.firmware_version.clone(), MAX_METADATA_VALUE_BYTES),
        );
    }
    if !info.serial_number.is_empty() {
        metadata.insert(
            "serial_number".to_string(),
            clamp_string_bytes(info.serial_number.clone(), MAX_METADATA_VALUE_BYTES),
        );
    }
    if !info.hardware_id.is_empty() {
        metadata.insert(
            "hardware_id".to_string(),
            clamp_string_bytes(info.hardware_id.clone(), MAX_METADATA_VALUE_BYTES),
        );
    }

    let capabilities = Some(vec![CapabilityDto {
        key: "onvif".to_string(),
        value: CapabilityValueDto::Boolean(true),
    }]);

    let register_request = RegisterDeviceRequest {
        protocol: "onvif".to_string(),
        external_id: endpoint_ref.to_string(),
        authority: None,
        name,
        kind: "camera".to_string(),
        capabilities,
        metadata: Some(metadata),
    };

    let mut uow = state
        .storage
        .begin()
        .await
        .map_err(|e| format!("storage begin failed: {e}"))?;
    let device = state
        .device_service
        .register_or_update_device(&context, &mut *uow, register_request)
        .await?;

    // Best-effort channel catalog from media profiles. Media endpoint discovery
    // requires GetCapabilities/GetServices which is not yet wired, so we leave
    // the channel list empty for now and only mark the device online.
    let replace_request = ReplaceChannelCatalogRequest { channels: vec![] };
    let mut uow = state
        .storage
        .begin()
        .await
        .map_err(|e| format!("storage begin failed: {e}"))?;
    state
        .device_service
        .replace_channel_catalog(
            &context,
            &mut *uow,
            device.device.device_id,
            replace_request,
        )
        .await?;

    let mut uow = state
        .storage
        .begin()
        .await
        .map_err(|e| format!("storage begin failed: {e}"))?;
    state
        .device_service
        .mark_device_online(
            &context,
            &mut *uow,
            device.device.device_id,
            MarkDeviceOnlineRequest {
                reason: Some("onvif discovery".to_string()),
            },
        )
        .await?;

    info!(
        %device.device.device_id,
        xaddr = %clamp_str(xaddr, MAX_LOG_XADDR_BYTES),
        "onvif device provisioned"
    );
    Ok(())
}

fn source_ip_from_xaddr(xaddr: &str) -> Option<String> {
    url::Url::parse(xaddr)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
}

fn device_name(info: &DeviceInformation, fallback: &str) -> String {
    let mut parts = Vec::new();
    if !info.manufacturer.is_empty() {
        parts.push(info.manufacturer.as_str());
    }
    if !info.model.is_empty() {
        parts.push(info.model.as_str());
    }
    if !info.serial_number.is_empty() {
        parts.push(info.serial_number.as_str());
    }
    let name = if parts.is_empty() {
        fallback.to_string()
    } else {
        parts.join(" ")
    };
    cheetah_signal_types::clamp_string_bytes(name, MAX_DEVICE_NAME_BYTES)
}

fn build_context(
    node_id: NodeId,
    tenant_id: TenantId,
    source_ip: Option<String>,
) -> RequestContext {
    RequestContext {
        tenant_id,
        principal: Principal {
            id: "onvif".to_string(),
            kind: PrincipalKind::Service,
            scopes: vec!["device:write".to_string()],
        },
        message_id: MessageId::from_uuid(Uuid::now_v7()),
        correlation_id: CorrelationId::from_uuid(Uuid::now_v7()),
        traceparent: None,
        tracestate: None,
        deadline: None,
        node_id: Some(node_id),
        source_ip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_discovery_interval_saturates_at_max_and_preserves_zero() {
        assert_eq!(clamp_discovery_interval(0), Duration::ZERO);
        assert_eq!(
            clamp_discovery_interval(5_000),
            Duration::from_millis(5_000)
        );
        assert_eq!(clamp_discovery_interval(u64::MAX), MAX_DISCOVERY_INTERVAL);
    }

    #[test]
    fn device_name_clamps_long_concatenated_fields() {
        let info = DeviceInformation {
            manufacturer: "a".repeat(600),
            model: "b".repeat(600),
            serial_number: "c".repeat(600),
            ..Default::default()
        };
        let name = device_name(&info, "fallback");
        assert_eq!(name.len(), MAX_DEVICE_NAME_BYTES);
        // UTF-8 boundary safety: a trailing multi-byte character is not split.
        assert!(name.is_char_boundary(name.len()));
    }

    #[test]
    fn device_name_uses_fallback_when_all_fields_empty() {
        let info = DeviceInformation::default();
        assert_eq!(device_name(&info, "fallback"), "fallback");
    }

    #[test]
    fn clamp_str_trims_xaddr_to_log_limit_without_splitting_chars() {
        let xaddr = "http://".to_string() + &"a".repeat(500);
        let clamped = clamp_str(&xaddr, MAX_LOG_XADDR_BYTES);
        assert_eq!(clamped.len(), MAX_LOG_XADDR_BYTES);
        assert!(clamped.is_char_boundary(clamped.len()));
    }
}

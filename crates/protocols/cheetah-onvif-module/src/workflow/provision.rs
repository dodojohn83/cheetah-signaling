//! ONVIF device provisioning workflow.

use crate::OnvifModuleError;
use crate::config::XAddrPolicy;
use crate::services::{
    get_capabilities_request, get_device_information_request, get_services_request,
};
use crate::types::{
    CapabilityKind, CapabilityProbeResult, DeviceInformation, OnvifEvent, ProvisioningStage,
};
use cheetah_signal_types::{DeviceId, IdGenerator, TenantId};
use std::collections::{HashMap, VecDeque};

fn message_id(id_generator: &dyn IdGenerator) -> String {
    format!("urn:uuid:{}", id_generator.generate_message_id())
}

/// Input to the provisioning workflow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProvisioningInput {
    /// A device was discovered with one or more XAddrs.
    Discovered {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Endpoint reference from WS-Discovery.
        endpoint_reference: String,
        /// Discovered XAddrs.
        xaddrs: Vec<String>,
    },
    /// Device information response received.
    DeviceInformation {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Parsed device information.
        info: DeviceInformation,
    },
    /// Capability probe result received.
    CapabilityProbed {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Capability kind.
        kind: CapabilityKind,
        /// Probe result.
        result: CapabilityProbeResult,
    },
    /// Services response received.
    ServicesReceived {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
    },
}

/// Output produced by the provisioning workflow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProvisioningOutput {
    /// Send an ONVIF request to a device.
    SendRequest {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Target XAddr.
        xaddr: String,
        /// SOAP action.
        action: &'static str,
        /// Request body (complete SOAP envelope).
        body: String,
    },
    /// Emit an event for downstream consumers.
    EmitEvent(OnvifEvent),
    /// Provisioning stage changed.
    StageChanged {
        /// Tenant identifier.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// New stage.
        stage: ProvisioningStage,
    },
}

/// Provisioning workflow error.
#[derive(Debug)]
pub enum ProvisionerError {
    /// No usable XAddr for the device.
    NoXAddr,
    /// Unknown device.
    UnknownDevice,
    /// Internal builder or parser failure.
    Internal(String),
}

impl std::fmt::Display for ProvisionerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoXAddr => f.write_str("no usable XAddr"),
            Self::UnknownDevice => f.write_str("unknown device"),
            Self::Internal(message) => write!(f, "internal: {message}"),
        }
    }
}

impl std::error::Error for ProvisionerError {}

impl From<OnvifModuleError> for ProvisionerError {
    fn from(e: OnvifModuleError) -> Self {
        Self::Internal(e.to_string())
    }
}

#[derive(Clone, Debug, Default)]
struct ProvisioningState {
    stage: ProvisioningStage,
    xaddrs: Vec<String>,
    #[allow(dead_code)]
    endpoint_reference: String,
    device_info: Option<DeviceInformation>,
    capabilities: HashMap<CapabilityKind, CapabilityProbeResult>,
}

/// Manages the ONVIF device provisioning workflow.
#[derive(Clone, Debug)]
pub struct Provisioner {
    states: HashMap<(TenantId, DeviceId), ProvisioningState>,
    xaddr_policy: XAddrPolicy,
    max_states: usize,
    eviction_order: VecDeque<(TenantId, DeviceId)>,
}

impl Default for Provisioner {
    fn default() -> Self {
        Self {
            states: HashMap::new(),
            xaddr_policy: XAddrPolicy::default(),
            max_states: 4096,
            eviction_order: VecDeque::new(),
        }
    }
}

impl Provisioner {
    /// Creates a new provisioner with default capacity and XAddr policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the XAddr SSRF policy applied to discovered and service addresses.
    pub fn with_xaddr_policy(mut self, policy: XAddrPolicy) -> Self {
        self.xaddr_policy = policy;
        self
    }

    /// Sets the maximum number of in-flight provisioning states.
    pub fn with_max_states(mut self, max: usize) -> Self {
        self.max_states = max.max(1);
        self
    }

    /// Removes a device's provisioning state, returning whether it existed.
    pub fn remove(&mut self, tenant_id: TenantId, device_id: DeviceId) -> bool {
        let key = (tenant_id, device_id);
        let removed = self.states.remove(&key).is_some();
        if removed {
            self.eviction_order.retain(|k| k != &key);
        }
        removed
    }

    /// Processes an input and returns the resulting outputs.
    ///
    /// `id_generator` is used to create message identifiers for outbound
    /// requests; callers must inject a deterministic generator in tests.
    pub fn process(
        &mut self,
        id_generator: &dyn IdGenerator,
        input: ProvisioningInput,
    ) -> Result<Vec<ProvisioningOutput>, ProvisionerError> {
        match input {
            ProvisioningInput::Discovered {
                tenant_id,
                device_id,
                endpoint_reference,
                xaddrs,
            } => {
                let xaddrs = self.xaddr_policy.filter(&xaddrs);
                if xaddrs.is_empty() {
                    return Err(ProvisionerError::NoXAddr);
                }
                if let Some(state) = self.states.get_mut(&(tenant_id, device_id)) {
                    state.xaddrs = xaddrs;
                    state.endpoint_reference = endpoint_reference;
                    return Ok(vec![]);
                }
                let key = (tenant_id, device_id);
                if self.states.len() >= self.max_states {
                    while let Some(oldest) = self.eviction_order.pop_front() {
                        if self.states.remove(&oldest).is_some() {
                            break;
                        }
                    }
                }
                self.eviction_order.push_back(key);
                let state = ProvisioningState {
                    stage: ProvisioningStage::PendingApproval,
                    xaddrs,
                    endpoint_reference,
                    ..Default::default()
                };
                self.states.insert(key, state);
                Ok(vec![ProvisioningOutput::StageChanged {
                    tenant_id,
                    device_id,
                    stage: ProvisioningStage::PendingApproval,
                }])
            }
            ProvisioningInput::DeviceInformation {
                tenant_id,
                device_id,
                info,
            } => {
                let Some(state) = self.states.get_mut(&(tenant_id, device_id)) else {
                    return Err(ProvisionerError::UnknownDevice);
                };
                state.device_info = Some(info.clone());
                state.stage = ProvisioningStage::Probing;
                let xaddr = state
                    .xaddrs
                    .first()
                    .cloned()
                    .ok_or(ProvisionerError::NoXAddr)?;
                let msg_id = message_id(id_generator);
                let mut outputs = vec![
                    ProvisioningOutput::EmitEvent(OnvifEvent::DeviceInformationReceived {
                        tenant_id,
                        device_id,
                        info,
                    }),
                    ProvisioningOutput::StageChanged {
                        tenant_id,
                        device_id,
                        stage: ProvisioningStage::Probing,
                    },
                ];
                outputs.push(ProvisioningOutput::SendRequest {
                    tenant_id,
                    device_id,
                    xaddr: xaddr.clone(),
                    action: "GetServices",
                    body: get_services_request(false, msg_id)?,
                });
                outputs.push(ProvisioningOutput::SendRequest {
                    tenant_id,
                    device_id,
                    xaddr,
                    action: "GetCapabilities",
                    body: get_capabilities_request(message_id(id_generator))?,
                });
                Ok(outputs)
            }
            ProvisioningInput::CapabilityProbed {
                tenant_id,
                device_id,
                kind,
                result,
            } => {
                let Some(state) = self.states.get_mut(&(tenant_id, device_id)) else {
                    return Err(ProvisionerError::UnknownDevice);
                };
                state.capabilities.insert(kind, result.clone());
                let previous_stage = state.stage;
                let stage = if state.capabilities.len() >= 4 {
                    ProvisioningStage::FetchingProfiles
                } else {
                    previous_stage
                };
                state.stage = stage;
                let mut outputs = vec![ProvisioningOutput::EmitEvent(
                    OnvifEvent::CapabilityProbed {
                        tenant_id,
                        device_id,
                        kind,
                        result,
                    },
                )];
                if stage != previous_stage {
                    outputs.push(ProvisioningOutput::StageChanged {
                        tenant_id,
                        device_id,
                        stage,
                    });
                }
                Ok(outputs)
            }
            ProvisioningInput::ServicesReceived {
                tenant_id,
                device_id,
            } => {
                let Some(state) = self.states.get_mut(&(tenant_id, device_id)) else {
                    return Err(ProvisionerError::UnknownDevice);
                };
                state.stage = ProvisioningStage::FetchingProfiles;
                Ok(vec![ProvisioningOutput::StageChanged {
                    tenant_id,
                    device_id,
                    stage: ProvisioningStage::FetchingProfiles,
                }])
            }
        }
    }

    /// Starts provisioning for a device by sending `GetDeviceInformation`.
    ///
    /// `id_generator` is used to create the outbound message identifier.
    pub fn start(
        &mut self,
        id_generator: &dyn IdGenerator,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Vec<ProvisioningOutput>, ProvisionerError> {
        let state = self
            .states
            .get_mut(&(tenant_id, device_id))
            .ok_or(ProvisionerError::UnknownDevice)?;
        state.stage = ProvisioningStage::Probing;
        let xaddr = state
            .xaddrs
            .first()
            .cloned()
            .ok_or(ProvisionerError::NoXAddr)?;
        let body = get_device_information_request(message_id(id_generator))?;
        Ok(vec![
            ProvisioningOutput::SendRequest {
                tenant_id,
                device_id,
                xaddr,
                action: "GetDeviceInformation",
                body,
            },
            ProvisioningOutput::StageChanged {
                tenant_id,
                device_id,
                stage: ProvisioningStage::Probing,
            },
        ])
    }

    /// Returns the current stage for a device, if known.
    pub fn stage(&self, tenant_id: TenantId, device_id: DeviceId) -> Option<ProvisioningStage> {
        self.states.get(&(tenant_id, device_id)).map(|s| s.stage)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::types::DeviceInformation;
    use cheetah_signal_types::{
        ChannelId, CorrelationId, DeliveryId, EndpointId, EventId, MediaBindingId,
        MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId, NodeInstanceId, OperationId,
        PluginId, ProtocolSessionId, TenantId, WebhookId,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use uuid::Uuid;

    #[derive(Default)]
    struct TestIdGenerator {
        counter: AtomicU64,
    }

    impl TestIdGenerator {
        fn next_uuid(&self) -> Uuid {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            let mut bytes = [0u8; 16];
            bytes[8..16].copy_from_slice(&n.to_be_bytes());
            Uuid::from_bytes(bytes)
        }
        fn next_u64(&self) -> u64 {
            self.counter.fetch_add(1, Ordering::SeqCst)
        }
    }

    impl IdGenerator for TestIdGenerator {
        fn generate_tenant_id(&self) -> TenantId {
            TenantId::from_uuid(self.next_uuid())
        }
        fn generate_device_id(&self) -> DeviceId {
            DeviceId::from_uuid(self.next_uuid())
        }
        fn generate_endpoint_id(&self) -> EndpointId {
            EndpointId::from_uuid(self.next_uuid())
        }
        fn generate_channel_id(&self) -> ChannelId {
            ChannelId::from_uuid(self.next_uuid())
        }
        fn generate_protocol_session_id(&self) -> ProtocolSessionId {
            ProtocolSessionId::from_uuid(self.next_uuid())
        }
        fn generate_platform_link_id(&self) -> cheetah_signal_types::PlatformLinkId {
            cheetah_signal_types::PlatformLinkId::from_uuid(self.next_uuid())
        }
        fn generate_media_session_id(&self) -> MediaSessionId {
            MediaSessionId::from_uuid(self.next_uuid())
        }
        fn generate_media_binding_id(&self) -> MediaBindingId {
            MediaBindingId::from_uuid(self.next_uuid())
        }
        fn generate_media_node_instance_epoch(&self) -> MediaNodeInstanceEpoch {
            MediaNodeInstanceEpoch(self.next_u64())
        }
        fn generate_operation_id(&self) -> OperationId {
            OperationId::from_uuid(self.next_uuid())
        }
        fn generate_node_id(&self) -> NodeId {
            NodeId::from_uuid(self.next_uuid())
        }
        fn generate_node_instance_id(&self) -> NodeInstanceId {
            NodeInstanceId::from_uuid(self.next_uuid())
        }
        fn generate_plugin_id(&self) -> PluginId {
            PluginId::from_uuid(self.next_uuid())
        }
        fn generate_event_id(&self) -> EventId {
            EventId::from_uuid(self.next_uuid())
        }
        fn generate_message_id(&self) -> MessageId {
            MessageId::from_uuid(self.next_uuid())
        }
        fn generate_correlation_id(&self) -> CorrelationId {
            CorrelationId::from_uuid(self.next_uuid())
        }
        fn generate_webhook_id(&self) -> WebhookId {
            WebhookId::from_uuid(self.next_uuid())
        }
        fn generate_delivery_id(&self) -> DeliveryId {
            DeliveryId::from_uuid(self.next_uuid())
        }
    }

    fn generator() -> Arc<TestIdGenerator> {
        Arc::new(TestIdGenerator::default())
    }

    #[test]
    fn discovered_sets_pending_approval() -> Result<(), ProvisionerError> {
        let mut p = Provisioner::new();
        let id_gen = generator();
        let tid = id_gen.generate_tenant_id();
        let did = id_gen.generate_device_id();
        let out = p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did,
                endpoint_reference: "urn:uuid:cam".to_string(),
                xaddrs: vec!["http://192.0.2.1/onvif".to_string()],
            },
        )?;
        assert!(out.iter().any(|o| matches!(
            o,
            ProvisioningOutput::StageChanged {
                stage: ProvisioningStage::PendingApproval,
                ..
            }
        )));
        Ok(())
    }

    #[test]
    fn rediscovered_preserves_existing_state() -> Result<(), ProvisionerError> {
        let mut p = Provisioner::new();
        let id_gen = generator();
        let tid = id_gen.generate_tenant_id();
        let did = id_gen.generate_device_id();
        p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did,
                endpoint_reference: "urn:uuid:cam".to_string(),
                xaddrs: vec!["http://192.0.2.1/onvif".to_string()],
            },
        )?;
        p.start(&*id_gen, tid, did)?;
        let out = p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did,
                endpoint_reference: "urn:uuid:cam2".to_string(),
                xaddrs: vec!["http://192.0.2.2/onvif".to_string()],
            },
        )?;
        assert!(out.is_empty());
        assert!(p.stage(tid, did) == Some(ProvisioningStage::Probing));
        Ok(())
    }

    #[test]
    fn start_sends_get_device_information() -> Result<(), ProvisionerError> {
        let mut p = Provisioner::new();
        let id_gen = generator();
        let tid = id_gen.generate_tenant_id();
        let did = id_gen.generate_device_id();
        p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did,
                endpoint_reference: "urn:uuid:cam".to_string(),
                xaddrs: vec!["http://192.0.2.1/onvif".to_string()],
            },
        )?;
        let out = p.start(&*id_gen, tid, did)?;
        let req = out.iter().find(|o| {
            matches!(
                o,
                ProvisioningOutput::SendRequest {
                    action: "GetDeviceInformation",
                    ..
                }
            )
        });
        assert!(req.is_some());
        if let Some(ProvisioningOutput::SendRequest { body, .. }) = req {
            assert!(
                body.contains("urn:uuid:"),
                "MessageID must be a urn:uuid IRI"
            );
        }
        Ok(())
    }

    #[test]
    fn device_information_triggers_services_and_capabilities() -> Result<(), ProvisionerError> {
        let mut p = Provisioner::new();
        let id_gen = generator();
        let tid = id_gen.generate_tenant_id();
        let did = id_gen.generate_device_id();
        p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did,
                endpoint_reference: "urn:uuid:cam".to_string(),
                xaddrs: vec!["http://192.0.2.1/onvif".to_string()],
            },
        )?;
        p.start(&*id_gen, tid, did)?;
        let out = p.process(
            &*id_gen,
            ProvisioningInput::DeviceInformation {
                tenant_id: tid,
                device_id: did,
                info: DeviceInformation {
                    manufacturer: "Acme".to_string(),
                    ..Default::default()
                },
            },
        )?;
        assert!(out.iter().any(|o| matches!(
            o,
            ProvisioningOutput::SendRequest {
                action: "GetServices",
                ..
            }
        )));
        assert!(out.iter().any(|o| matches!(
            o,
            ProvisioningOutput::SendRequest {
                action: "GetCapabilities",
                ..
            }
        )));
        Ok(())
    }

    #[test]
    fn discovered_rejects_blocked_xaddrs() {
        let mut p = Provisioner::new();
        let id_gen = generator();
        let tid = id_gen.generate_tenant_id();
        let did = id_gen.generate_device_id();
        let out = p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did,
                endpoint_reference: "urn:uuid:cam".to_string(),
                xaddrs: vec!["http://127.0.0.1/onvif".to_string()],
            },
        );
        assert!(matches!(out, Err(ProvisionerError::NoXAddr)));
    }

    #[test]
    fn eviction_bounds_state_map() -> Result<(), ProvisionerError> {
        let mut p = Provisioner::new().with_max_states(1);
        let id_gen = generator();
        let tid = id_gen.generate_tenant_id();
        let did1 = id_gen.generate_device_id();
        let did2 = id_gen.generate_device_id();
        p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did1,
                endpoint_reference: "urn:uuid:cam1".to_string(),
                xaddrs: vec!["http://192.0.2.1/onvif".to_string()],
            },
        )?;
        p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did2,
                endpoint_reference: "urn:uuid:cam2".to_string(),
                xaddrs: vec!["http://192.0.2.2/onvif".to_string()],
            },
        )?;
        assert!(p.stage(tid, did1).is_none());
        assert!(p.stage(tid, did2).is_some());
        Ok(())
    }

    #[test]
    fn remove_drops_state() -> Result<(), ProvisionerError> {
        let mut p = Provisioner::new();
        let id_gen = generator();
        let tid = id_gen.generate_tenant_id();
        let did = id_gen.generate_device_id();
        p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did,
                endpoint_reference: "urn:uuid:cam".to_string(),
                xaddrs: vec!["http://192.0.2.1/onvif".to_string()],
            },
        )?;
        assert!(p.remove(tid, did));
        assert!(!p.remove(tid, did));
        Ok(())
    }

    #[test]
    fn capability_probed_does_not_emit_duplicate_stage_changed() -> Result<(), ProvisionerError> {
        let mut p = Provisioner::new();
        let id_gen = generator();
        let tid = id_gen.generate_tenant_id();
        let did = id_gen.generate_device_id();
        p.process(
            &*id_gen,
            ProvisioningInput::Discovered {
                tenant_id: tid,
                device_id: did,
                endpoint_reference: "urn:uuid:cam".to_string(),
                xaddrs: vec!["http://192.0.2.1/onvif".to_string()],
            },
        )?;
        p.start(&*id_gen, tid, did)?;
        let mut seen = 0;
        for _ in 0..3 {
            let out = p.process(
                &*id_gen,
                ProvisioningInput::CapabilityProbed {
                    tenant_id: tid,
                    device_id: did,
                    kind: CapabilityKind::Device,
                    result: CapabilityProbeResult::Unsupported,
                },
            )?;
            if out
                .iter()
                .any(|o| matches!(o, ProvisioningOutput::StageChanged { .. }))
            {
                seen += 1;
            }
        }
        assert!(
            seen <= 1,
            "expected at most one stage change for repeated probes"
        );
        Ok(())
    }
}

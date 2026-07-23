//! Device application service.

use crate::dto::{
    DeviceDto, MarkDeviceOfflineRequest, MarkDeviceOnlineRequest, RegisterDeviceRequest,
    RegisterDeviceResult, ReplaceChannelCatalogRequest, RetireDeviceRequest,
    UpdateDeviceCapabilitiesRequest,
};
use cheetah_domain::{
    Capability, Channel, ChannelKind, ChannelStatus, Device, DeviceKind, DeviceLifecycle,
    DomainError, DomainEvent, Protocol, StreamProfile, UnitOfWork,
};
use cheetah_signal_types::{
    ChannelId, Clock, DeviceId, Event, IdGenerator, RequestContext, ResourceId, ResourceKind,
    ResourceRef, TenantId,
};
use std::collections::HashSet;

/// Maximum number of channels allowed per device.
const MAX_CHANNELS: usize = 1024;

/// Application service for device lifecycle.
#[derive(Clone)]
pub struct DeviceService {
    clock: std::sync::Arc<dyn Clock>,
    id_generator: std::sync::Arc<dyn IdGenerator>,
}

impl DeviceService {
    /// Creates a new device service.
    pub fn new(
        clock: std::sync::Arc<dyn Clock>,
        id_generator: std::sync::Arc<dyn IdGenerator>,
    ) -> Self {
        Self {
            clock,
            id_generator,
        }
    }

    /// Registers a new device or updates an existing one by external identity.
    pub async fn register_or_update_device(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        request: RegisterDeviceRequest,
    ) -> crate::Result<RegisterDeviceResult> {
        let tenant_id = context.tenant_id;
        let protocol = request
            .protocol
            .parse::<Protocol>()
            .map_err(crate::SignalError::from)?;
        let kind = request
            .kind
            .parse::<DeviceKind>()
            .map_err(crate::SignalError::from)?;
        let external_id = cheetah_signal_types::ProtocolIdentity::new(request.external_id)?;
        let authority = request.authority.unwrap_or_default();
        let capabilities = if let Some(dtos) = request.capabilities {
            dtos.into_iter()
                .map(Capability::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(crate::SignalError::from)?
        } else {
            Vec::new()
        };
        let metadata = request.metadata.unwrap_or_default();
        let name = request.name;

        let (device, event, created) = if let Some(mut existing) = uow
            .device_repository()
            .get_by_external_id(tenant_id, protocol, external_id.clone())
            .await?
        {
            let event = existing.update(
                self.clock.as_ref(),
                Some(name.clone()),
                Some(kind),
                Some(protocol),
                Some(external_id.clone()),
                Some(authority.clone()),
                Some(capabilities.clone()),
                Some(metadata.clone()),
            )?;
            (existing, event, false)
        } else {
            let device_id = self.id_generator.generate_device_id();
            let (device, event) = Device::new(
                self.clock.as_ref(),
                tenant_id,
                device_id,
                protocol,
                external_id,
                authority,
                name,
                kind,
                capabilities,
                metadata,
            )
            .map_err(crate::SignalError::from)?;
            (device, event, true)
        };

        uow.device_repository().save(&device).await?;
        let event = wrap_event(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            tenant_id,
            device_resource_ref(tenant_id, device.device_id()),
            device.revision().0,
            event,
        );
        uow.outbox().append(event).await?;
        uow.commit().await?;

        Ok(RegisterDeviceResult {
            device: DeviceDto::from(&device),
            created,
        })
    }

    /// Marks a device as online.
    pub async fn mark_device_online(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        device_id: DeviceId,
        request: MarkDeviceOnlineRequest,
    ) -> crate::Result<DeviceDto> {
        let tenant_id = context.tenant_id;
        let mut device = uow
            .device_repository()
            .get(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "device",
                    device_id.to_string(),
                ))
            })?;
        let event = device
            .mark_online(self.clock.as_ref(), request.reason)
            .map_err(crate::SignalError::from)?;
        uow.device_repository().save(&device).await?;
        let event = wrap_event(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            tenant_id,
            device_resource_ref(tenant_id, device.device_id()),
            device.revision().0,
            event,
        );
        uow.outbox().append(event).await?;
        uow.commit().await?;
        Ok(DeviceDto::from(&device))
    }

    /// Marks a device as offline.
    pub async fn mark_device_offline(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        device_id: DeviceId,
        request: MarkDeviceOfflineRequest,
    ) -> crate::Result<DeviceDto> {
        let tenant_id = context.tenant_id;
        let mut device = uow
            .device_repository()
            .get(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "device",
                    device_id.to_string(),
                ))
            })?;
        let event = device
            .mark_offline(self.clock.as_ref(), request.reason)
            .map_err(crate::SignalError::from)?;
        uow.device_repository().save(&device).await?;
        let event = wrap_event(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            tenant_id,
            device_resource_ref(tenant_id, device.device_id()),
            device.revision().0,
            event,
        );
        uow.outbox().append(event).await?;
        uow.commit().await?;
        Ok(DeviceDto::from(&device))
    }

    /// Replaces the full channel catalog for a device.
    pub async fn replace_channel_catalog(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        device_id: DeviceId,
        request: ReplaceChannelCatalogRequest,
    ) -> crate::Result<DeviceDto> {
        let tenant_id = context.tenant_id;
        let mut device = uow
            .device_repository()
            .get(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "device",
                    device_id.to_string(),
                ))
            })?;

        if device.lifecycle() == DeviceLifecycle::Retired {
            return Err(crate::SignalError::from(DomainError::invalid_transition(
                "Device",
                "Retired",
                "replace_channel_catalog",
            )));
        }

        if request.channels.len() > MAX_CHANNELS {
            return Err(crate::SignalError::from(DomainError::invalid_argument(
                "channel catalog exceeds maximum allowed channels",
            )));
        }

        let mut incoming_ids = HashSet::with_capacity(request.channels.len());
        for descriptor in &request.channels {
            if let Some(id) = &descriptor.id {
                let channel_id = id.parse::<ChannelId>()?;
                incoming_ids.insert(channel_id);
            }
        }

        let existing = uow
            .channel_repository()
            .list_by_device(tenant_id, device_id)
            .await?;

        // Remove channels that are no longer in the catalog.
        for channel in existing {
            if !incoming_ids.contains(&channel.channel_id()) {
                let channel_id = channel.channel_id();
                let revision = channel.revision();
                let event = channel.remove();
                let event = wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    channel_resource_ref(tenant_id, device_id, channel_id),
                    revision.0,
                    event,
                );
                uow.outbox().append(event).await?;
                uow.channel_repository()
                    .remove(tenant_id, device_id, channel_id, revision)
                    .await?;
            }
        }

        // Create or update channels.
        for descriptor in request.channels {
            let channel_id = if let Some(id) = descriptor.id {
                id.parse::<ChannelId>()?
            } else {
                self.id_generator.generate_channel_id()
            };
            let kind = descriptor
                .kind
                .parse::<ChannelKind>()
                .map_err(crate::SignalError::from)?;
            let status = descriptor
                .status
                .map(|s| s.parse::<ChannelStatus>())
                .transpose()
                .map_err(crate::SignalError::from)?;
            let stream_profiles = descriptor
                .stream_profiles
                .into_iter()
                .map(StreamProfile::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(crate::SignalError::from)?;
            let ptz_capabilities = descriptor
                .ptz_capabilities
                .map(cheetah_domain::PtzCapabilities::from)
                .unwrap_or_default();
            let metadata = descriptor.metadata.unwrap_or_default();
            let name = descriptor.name;
            let enabled = descriptor.enabled;

            let (channel, event) = if let Some(mut existing) = uow
                .channel_repository()
                .get(tenant_id, device_id, channel_id)
                .await?
            {
                let event = existing.update(
                    self.clock.as_ref(),
                    Some(kind),
                    Some(name.clone()),
                    Some(enabled),
                    status,
                    Some(stream_profiles.clone()),
                    Some(ptz_capabilities.clone()),
                    Some(metadata.clone()),
                )?;
                (existing, event)
            } else {
                Channel::new(
                    self.clock.as_ref(),
                    tenant_id,
                    device_id,
                    channel_id,
                    kind,
                    name,
                    enabled,
                    status,
                    stream_profiles,
                    ptz_capabilities,
                    metadata,
                )
                .map_err(crate::SignalError::from)?
            };
            uow.channel_repository().save(&channel).await?;
            let event = wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                tenant_id,
                channel_resource_ref(tenant_id, device_id, channel.channel_id()),
                channel.revision().0,
                event,
            );
            uow.outbox().append(event).await?;
        }

        // Touch device revision and emit DeviceUpdated.
        let event = device.touch(self.clock.as_ref());
        uow.device_repository().save(&device).await?;
        let event = wrap_event(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            tenant_id,
            device_resource_ref(tenant_id, device.device_id()),
            device.revision().0,
            event,
        );
        uow.outbox().append(event).await?;
        uow.commit().await?;

        Ok(DeviceDto::from(&device))
    }

    /// Updates device capabilities and metadata.
    ///
    /// `expected_revision` is the client-observed revision from `If-Match` /
    /// `ETag`. A mismatch returns [`DomainError::ConcurrentModification`].
    pub async fn update_device_capabilities(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        device_id: DeviceId,
        expected_revision: cheetah_signal_types::Revision,
        request: UpdateDeviceCapabilitiesRequest,
    ) -> crate::Result<DeviceDto> {
        let tenant_id = context.tenant_id;
        let mut device = uow
            .device_repository()
            .get(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "device",
                    device_id.to_string(),
                ))
            })?;

        let current = device.revision();
        if current != expected_revision {
            return Err(crate::SignalError::new(
                cheetah_signal_types::SignalErrorKind::FailedPrecondition,
                format!(
                    "device revision mismatch: If-Match {}, current {}",
                    expected_revision.0, current.0
                ),
            ));
        }

        let capabilities = if let Some(dtos) = request.capabilities {
            Some(
                dtos.into_iter()
                    .map(Capability::try_from)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(crate::SignalError::from)?,
            )
        } else {
            None
        };
        let metadata = request.metadata;

        let event = device
            .update(
                self.clock.as_ref(),
                None,
                None,
                None,
                None,
                None,
                capabilities,
                metadata,
            )
            .map_err(crate::SignalError::from)?;
        uow.device_repository().save(&device).await?;
        let event = wrap_event(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            tenant_id,
            device_resource_ref(tenant_id, device.device_id()),
            device.revision().0,
            event,
        );
        uow.outbox().append(event).await?;
        uow.commit().await?;
        Ok(DeviceDto::from(&device))
    }

    /// Retires a device.
    pub async fn retire_device(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        device_id: DeviceId,
        _request: RetireDeviceRequest,
    ) -> crate::Result<DeviceDto> {
        let tenant_id = context.tenant_id;
        let mut device = uow
            .device_repository()
            .get(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(cheetah_domain::DomainError::not_found(
                    "device",
                    device_id.to_string(),
                ))
            })?;
        let event = device
            .retire(self.clock.as_ref())
            .map_err(crate::SignalError::from)?;
        uow.device_repository().save(&device).await?;
        let event = wrap_event(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            tenant_id,
            device_resource_ref(tenant_id, device.device_id()),
            device.revision().0,
            event,
        );
        uow.outbox().append(event).await?;
        uow.commit().await?;
        Ok(DeviceDto::from(&device))
    }
}

impl std::fmt::Debug for DeviceService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceService").finish_non_exhaustive()
    }
}

fn wrap_event(
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
    context: &RequestContext,
    tenant_id: TenantId,
    aggregate_ref: ResourceRef,
    aggregate_sequence: u64,
    payload: DomainEvent,
) -> Event<DomainEvent> {
    Event::new(
        id_generator,
        clock,
        context,
        tenant_id,
        aggregate_ref,
        aggregate_sequence,
        payload,
    )
}

fn device_resource_ref(tenant_id: TenantId, device_id: DeviceId) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    }
}

fn channel_resource_ref(
    tenant_id: TenantId,
    _device_id: DeviceId,
    channel_id: ChannelId,
) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Channel,
        id: ResourceId::Channel(channel_id),
    }
}

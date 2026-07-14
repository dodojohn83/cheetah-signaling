//! Device owner lease and resolver implementation.

use cheetah_domain::{Clock, DeviceOwnerResolver, DomainError, OwnerInfo};
use cheetah_signal_types::{DeviceId, DurationMs, NodeId, OwnerEpoch, TenantId, UtcTimestamp};
use cheetah_storage_api::{OwnerRepository, StorageError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

/// Manages device owner leases for a single node.
pub struct OwnerLeaseService {
    repository: Arc<tokio::sync::Mutex<dyn OwnerRepository>>,
    clock: Arc<dyn Clock>,
    this_node: NodeId,
    lease_duration: DurationMs,
}

impl OwnerLeaseService {
    /// Creates a new lease service.
    pub fn new(
        repository: Arc<tokio::sync::Mutex<dyn OwnerRepository>>,
        clock: Arc<dyn Clock>,
        this_node: NodeId,
        lease_duration: DurationMs,
    ) -> Self {
        Self {
            repository,
            clock,
            this_node,
            lease_duration,
        }
    }

    fn lease_until(&self) -> Result<UtcTimestamp, StorageError> {
        self.clock
            .now_wall()
            .checked_add(self.lease_duration)
            .ok_or_else(|| StorageError::internal("lease timestamp overflow"))
    }

    /// Acquires (or re-acquires) ownership of `device_id` for this node.
    pub async fn acquire(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<OwnerInfo, StorageError> {
        let lease_until = self.lease_until()?;
        let mut repo = self.repository.lock().await;
        let owner = repo
            .acquire(tenant_id, device_id, self.this_node, lease_until)
            .await?;
        info!(
            tenant_id = %tenant_id.as_uuid(),
            device_id = %device_id.as_uuid(),
            node_id = %self.this_node.as_uuid(),
            epoch = owner.owner_epoch.0,
            "acquired device lease"
        );
        Ok(owner)
    }

    /// Renews the lease for `device_id` if this node still owns it.
    pub async fn renew(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Option<OwnerInfo>, StorageError> {
        let lease_until = self.lease_until()?;
        let mut repo = self.repository.lock().await;
        repo.renew(tenant_id, device_id, self.this_node, lease_until)
            .await
    }

    /// Renews leases for multiple devices, returning each result in order.
    pub async fn batch_renew(
        &self,
        devices: &[(TenantId, DeviceId)],
    ) -> Vec<Result<Option<OwnerInfo>, StorageError>> {
        let mut results = Vec::with_capacity(devices.len());
        for (tenant_id, device_id) in devices {
            results.push(self.renew(*tenant_id, *device_id).await);
        }
        results
    }

    /// Releases ownership for `device_id`.
    pub async fn release(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        epoch: OwnerEpoch,
    ) -> Result<(), StorageError> {
        let mut repo = self.repository.lock().await;
        repo.release(tenant_id, device_id, self.this_node, epoch)
            .await
    }
}

impl std::fmt::Debug for OwnerLeaseService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerLeaseService")
            .field("this_node", &self.this_node)
            .finish_non_exhaustive()
    }
}

struct CacheEntry {
    owner: OwnerInfo,
    valid_until: UtcTimestamp,
}

/// Caching owner resolver backed by an `OwnerRepository`.
pub struct CachingDeviceOwnerResolver {
    repository: Arc<dyn OwnerRepository>,
    clock: Arc<dyn Clock>,
    cache_ttl: DurationMs,
    cache: Mutex<HashMap<(TenantId, DeviceId), CacheEntry>>,
}

impl CachingDeviceOwnerResolver {
    /// Creates a new caching resolver.
    pub fn new(
        repository: Arc<dyn OwnerRepository>,
        clock: Arc<dyn Clock>,
        cache_ttl: DurationMs,
    ) -> Self {
        Self {
            repository,
            clock,
            cache_ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn cache_valid_until(&self, owner: &OwnerInfo, cached_at: UtcTimestamp) -> UtcTimestamp {
        let ttl_deadline = cached_at.checked_add(self.cache_ttl).unwrap_or(cached_at);
        match owner.lease_until {
            Some(lease) => lease.min(ttl_deadline),
            None => ttl_deadline,
        }
    }
}

#[async_trait::async_trait]
impl DeviceOwnerResolver for CachingDeviceOwnerResolver {
    async fn resolve(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Option<OwnerInfo>, DomainError> {
        let now = self.clock.now_wall();

        {
            let cache = self
                .cache
                .lock()
                .map_err(|e| DomainError::internal(format!("owner cache poisoned: {e}")))?;
            if let Some(entry) = cache.get(&(tenant_id, device_id))
                && entry.valid_until > now
            {
                return Ok(Some(entry.owner.clone()));
            }
        }

        let owner = self
            .repository
            .get(tenant_id, device_id)
            .await
            .map_err(DomainError::from)?;

        if let Some(ref o) = owner {
            let cached_at = self.clock.now_wall();
            let valid_until = self.cache_valid_until(o, cached_at);
            if valid_until > now {
                let mut cache = self
                    .cache
                    .lock()
                    .map_err(|e| DomainError::internal(format!("owner cache poisoned: {e}")))?;
                cache.insert(
                    (tenant_id, device_id),
                    CacheEntry {
                        owner: o.clone(),
                        valid_until,
                    },
                );
                Ok(owner)
            } else {
                warn!(
                    tenant_id = %tenant_id.as_uuid(),
                    device_id = %device_id.as_uuid(),
                    "resolved owner lease already expired"
                );
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

impl std::fmt::Debug for CachingDeviceOwnerResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachingDeviceOwnerResolver")
            .finish_non_exhaustive()
    }
}

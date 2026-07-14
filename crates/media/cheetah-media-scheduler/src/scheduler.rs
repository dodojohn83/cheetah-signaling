//! Media node scheduling.

use crate::config::SchedulerConfig;
use crate::error::SchedulerError;
use crate::model::{MediaNode, MediaNodeHealth, NodeStatus};
use crate::registry::MediaNodeRegistry;
use cheetah_domain::MediaRequirements;
use cheetah_signal_types::{Clock, MediaBindingId, NodeId, TenantId};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hasher};
use std::sync::{Arc, Mutex};

/// Schedules media bindings onto registered media nodes.
#[async_trait::async_trait]
pub trait MediaScheduler: Send + Sync {
    /// Selects a node for the given requirements.
    async fn schedule(
        &self,
        tenant_id: TenantId,
        requirements: &MediaRequirements,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError>;

    /// Reserves capacity on the selected node for a media binding.
    async fn reserve(
        &self,
        node_id: NodeId,
        tenant_id: TenantId,
        binding_id: MediaBindingId,
        requirements: &MediaRequirements,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError>;

    /// Releases a previously reserved binding.
    async fn release(
        &self,
        tenant_id: TenantId,
        binding_id: MediaBindingId,
        clock: &dyn Clock,
    ) -> Result<(), SchedulerError>;
}

/// Least-loaded scheduler with stable scoring and per-session affinity.
pub struct LeastLoadedScheduler {
    registry: Arc<dyn MediaNodeRegistry>,
    config: SchedulerConfig,
    reservations: Mutex<BTreeMap<(TenantId, MediaBindingId), NodeId>>,
    affinity: Mutex<BTreeMap<(TenantId, String), NodeId>>,
    affinity_count: Mutex<BTreeMap<(TenantId, String), usize>>,
    binding_session: Mutex<BTreeMap<(TenantId, MediaBindingId), String>>,
}

impl LeastLoadedScheduler {
    /// Creates a scheduler backed by the given registry.
    pub fn new(registry: Arc<dyn MediaNodeRegistry>, config: SchedulerConfig) -> Self {
        Self {
            registry,
            config,
            reservations: Mutex::new(BTreeMap::new()),
            affinity: Mutex::new(BTreeMap::new()),
            affinity_count: Mutex::new(BTreeMap::new()),
            binding_session: Mutex::new(BTreeMap::new()),
        }
    }
}

impl std::fmt::Debug for LeastLoadedScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeastLoadedScheduler")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl MediaScheduler for LeastLoadedScheduler {
    async fn schedule(
        &self,
        tenant_id: TenantId,
        requirements: &MediaRequirements,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let candidates = self.registry.list_active(clock).await;
        if candidates.is_empty() {
            return Err(SchedulerError::NoNode(
                "no registered media nodes".to_string(),
            ));
        }

        if let Some(session_id) = requirements.media_session_id.as_ref() {
            let node_id = self
                .affinity
                .lock()
                .map_err(|_| SchedulerError::InvalidArgument("affinity lock poisoned".to_string()))?
                .get(&(tenant_id, session_id.clone()))
                .copied();
            if let Some(node_id) = node_id
                && let Some(node) = candidates.iter().find(|n| n.node_id == node_id)
                && is_eligible_for_affinity(node, requirements)
            {
                return Ok(node.clone());
            }
        }

        let mut scored: Vec<(MediaNode, f64)> = candidates
            .into_iter()
            .filter(|n| n.status == NodeStatus::Active && !n.draining)
            .filter(|n| n.health != MediaNodeHealth::Unhealthy)
            .filter(|n| matches_capability(n, requirements))
            .filter(has_capacity)
            .map(|n| {
                let score = score_node(&n, requirements, &self.config);
                (n, score)
            })
            .filter(|(_, score)| score.is_finite())
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(self.config.max_candidates);

        scored
            .into_iter()
            .next()
            .map(|(n, _)| n)
            .ok_or_else(|| SchedulerError::NoNode(format_no_candidate_reason(requirements)))
    }

    async fn reserve(
        &self,
        node_id: NodeId,
        tenant_id: TenantId,
        binding_id: MediaBindingId,
        requirements: &MediaRequirements,
        clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let node = self
            .registry
            .reserve(node_id, tenant_id, binding_id, clock)
            .await?;
        let mut reservations = self.reservations.lock().map_err(|_| {
            SchedulerError::InvalidArgument("reservation lock poisoned".to_string())
        })?;
        reservations.insert((tenant_id, binding_id), node_id);
        drop(reservations);

        if let Some(session_id) = requirements.media_session_id.as_ref() {
            let key = (tenant_id, session_id.clone());
            self.affinity
                .lock()
                .map_err(|_| SchedulerError::InvalidArgument("affinity lock poisoned".to_string()))?
                .insert(key.clone(), node_id);
            let mut counts = self.affinity_count.lock().map_err(|_| {
                SchedulerError::InvalidArgument("affinity count lock poisoned".to_string())
            })?;
            *counts.entry(key).or_insert(0) += 1;
            drop(counts);
            self.binding_session
                .lock()
                .map_err(|_| {
                    SchedulerError::InvalidArgument("binding session lock poisoned".to_string())
                })?
                .insert((tenant_id, binding_id), session_id.clone());
        }
        Ok(node)
    }

    async fn release(
        &self,
        tenant_id: TenantId,
        binding_id: MediaBindingId,
        clock: &dyn Clock,
    ) -> Result<(), SchedulerError> {
        let session_id = self
            .binding_session
            .lock()
            .map_err(|_| {
                SchedulerError::InvalidArgument("binding session lock poisoned".to_string())
            })?
            .remove(&(tenant_id, binding_id));

        if let Some(session_id) = session_id {
            let key = (tenant_id, session_id);
            let mut counts = self.affinity_count.lock().map_err(|_| {
                SchedulerError::InvalidArgument("affinity count lock poisoned".to_string())
            })?;
            if let Some(count) = counts.get_mut(&key) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(&key);
                    drop(counts);
                    self.affinity
                        .lock()
                        .map_err(|_| {
                            SchedulerError::InvalidArgument("affinity lock poisoned".to_string())
                        })?
                        .remove(&key);
                }
            }
        }

        let node_id = {
            let mut reservations = self.reservations.lock().map_err(|_| {
                SchedulerError::InvalidArgument("reservation lock poisoned".to_string())
            })?;
            reservations.remove(&(tenant_id, binding_id))
        };
        if let Some(node_id) = node_id {
            self.registry
                .release(node_id, tenant_id, binding_id, clock)
                .await?;
        }
        Ok(())
    }
}

fn is_eligible_for_affinity(node: &MediaNode, requirements: &MediaRequirements) -> bool {
    matches_capability(node, requirements)
        && zone_matches(node, requirements)
        && node.health != MediaNodeHealth::Unhealthy
        && node.status != NodeStatus::Left
}

fn matches_capability(node: &MediaNode, requirements: &MediaRequirements) -> bool {
    node.capabilities.iter().any(|cap| {
        cap.protocol == requirements.protocol
            && (requirements.operation.is_empty()
                || cap.operations.contains(&requirements.operation))
            && constraints_satisfy(&cap.constraints, &requirements.required_constraints)
            && constraints_satisfy(&cap.constraints, &requirements.tenant_constraints)
            && codec_compatible(cap, requirements)
    })
}

fn constraints_satisfy(
    offered: &BTreeMap<String, String>,
    required: &BTreeMap<String, String>,
) -> bool {
    required.iter().all(|(k, v)| offered.get(k) == Some(v))
}

fn codec_compatible(
    capability: &crate::model::MediaCapability,
    requirements: &MediaRequirements,
) -> bool {
    if requirements.codecs.is_empty() {
        return true;
    }
    let supported = capability
        .constraints
        .get("codecs")
        .map(|s| s.split(',').map(str::trim).collect::<BTreeSet<_>>())
        .unwrap_or_default();
    requirements
        .codecs
        .iter()
        .any(|c| supported.contains(c.as_str()))
}

fn zone_matches(node: &MediaNode, requirements: &MediaRequirements) -> bool {
    if let Some(zone) = requirements.zone.as_ref() {
        node.zone == *zone || node.region == *zone
    } else {
        true
    }
}

fn has_capacity(node: &MediaNode) -> bool {
    node.has_capacity()
}

fn score_node(node: &MediaNode, requirements: &MediaRequirements, config: &SchedulerConfig) -> f64 {
    let available_sessions = node.available_sessions() as f64;
    let session_capacity = node.capacity.max_sessions.max(1) as f64;
    let session_score = available_sessions / session_capacity;

    let max_cpu = if node.capacity.max_cpu_percent == 0 {
        100
    } else {
        node.capacity.max_cpu_percent
    } as f64;
    let cpu_score = 1.0 - (node.load as f64 / max_cpu).min(1.0);

    let bandwidth_score = 1.0;

    let zone_score = if zone_matches(node, requirements) {
        1.0
    } else {
        0.0
    };

    let random_score = stable_random(
        requirements.media_session_id.as_deref(),
        &node.node_id.to_string(),
    );

    session_score * config.available_sessions_weight
        + bandwidth_score * config.bandwidth_weight
        + cpu_score * config.cpu_weight
        + zone_score * config.zone_affinity_weight
        + random_score * config.stable_random_weight
}

fn stable_random(media_session_id: Option<&str>, node_id: &str) -> f64 {
    let seed = media_session_id.unwrap_or(node_id);
    let mut hasher = DefaultHasher::new();
    hasher.write(seed.as_bytes());
    hasher.write(node_id.as_bytes());
    let value = hasher.finish();
    (value as f64) / (u64::MAX as f64)
}

fn format_no_candidate_reason(requirements: &MediaRequirements) -> String {
    format!(
        "no node satisfies protocol={} operation={} zone={:?} constraints={:?}",
        requirements.protocol,
        requirements.operation,
        requirements.zone,
        requirements.required_constraints
    )
}

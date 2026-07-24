//! Multi-link cascade manager (`GB4-CAS-006`).
//!
//! [`Gb28181Cascade`] is a *single* upstream state machine. Real deployments
//! federate to several upstream platforms and accept several downstream
//! platforms at once. [`CascadeManager`] owns one independent
//! [`Gb28181Cascade`] per upstream link plus lightweight downstream enrollment
//! records, and layers the cross-link control-plane policy on top:
//!
//! - **multi-upstream isolation** — each link has its own Call-ID/CSeq/auth
//!   and state; a failure or malformed message on one link never mutates or
//!   blocks another link (`process`/`tick_all` are per-link);
//! - **downstream enrollment** — downstream platforms are validated and stored
//!   with their ACL;
//! - **platform identity validation** — inbound requests are matched against
//!   the enrolled remote *platform* identity, keeping platform identities
//!   distinct from device/channel identities;
//! - **ACL enforcement** — catalog sharing, control and media bridging are
//!   gated by the link's [`PlatformAcl`];
//! - **loop detection and hop limits** — bridge routing rejects revisited
//!   platforms, explicitly denied platforms and paths exceeding
//!   [`MAX_CASCADE_HOPS`];
//! - **unique control ownership** — at most one link may hold control of a
//!   given resource at a time.
//!
//! The manager performs no I/O; callers pump [`CascadeInput`] in and dispatch
//! the returned [`CascadeOutput`]s just as with a single machine.

use super::{
    CascadeConfig, CascadeCredentialProvider, CascadeError, CascadeEvent, CascadeInput,
    CascadeOutput, Gb28181Cascade,
};
use crate::types::DomainId;
use cheetah_domain::{
    GbPlatformLink, MAX_CASCADE_HOPS, PlatformAcl, PlatformDirection, detect_loop,
};
use cheetah_gb28181_core::SipUri;
use cheetah_signal_types::PlatformLinkId;
use std::collections::BTreeMap;

/// Errors returned by cross-link cascade routing and enrollment.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CascadeRoutingError {
    /// No link is registered for the supplied identifier.
    #[error("unknown platform link")]
    UnknownLink,
    /// The link direction is incompatible with the requested operation.
    #[error("wrong link direction: expected {expected}")]
    WrongDirection {
        /// The direction the operation required.
        expected: &'static str,
    },
    /// Another link is already registered for the same remote identity.
    #[error("duplicate platform link for remote identity")]
    DuplicateRemote,
    /// The presented platform identity does not match the enrolled link.
    #[error("platform identity mismatch")]
    IdentityMismatch,
    /// Routing the request would revisit a platform already on the path.
    #[error("routing loop detected")]
    LoopDetected,
    /// The cascade path already reached [`MAX_CASCADE_HOPS`].
    #[error("hop limit exceeded")]
    HopLimitExceeded,
    /// The link's ACL forbids the requested action.
    #[error("access denied by ACL")]
    AclDenied,
    /// A different link already owns control of the resource.
    #[error("control already owned by another link")]
    ControlConflict,
    /// A single-link cascade error surfaced while building or driving a link.
    #[error(transparent)]
    Cascade(#[from] CascadeError),
}

/// A single upstream link managed by the [`CascadeManager`].
struct ManagedUpstream<P: CascadeCredentialProvider> {
    remote_identity: String,
    acl: PlatformAcl,
    machine: Gb28181Cascade<P>,
}

/// An enrolled downstream platform.
#[derive(Clone, Debug)]
struct DownstreamEnrollment {
    link_id: PlatformLinkId,
    acl: PlatformAcl,
}

/// Owns and coordinates multiple cascade links for a single tenant/platform.
pub struct CascadeManager<P: CascadeCredentialProvider> {
    local_identity: String,
    upstreams: BTreeMap<PlatformLinkId, ManagedUpstream<P>>,
    downstreams: BTreeMap<String, DownstreamEnrollment>,
    control_owner: BTreeMap<String, PlatformLinkId>,
}

impl<P: CascadeCredentialProvider> std::fmt::Debug for CascadeManager<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CascadeManager")
            .field("local_identity", &self.local_identity)
            .field("upstreams", &self.upstreams.len())
            .field("downstreams", &self.downstreams.len())
            .field("control_owner", &self.control_owner.len())
            .finish()
    }
}

impl<P: CascadeCredentialProvider> CascadeManager<P> {
    /// Creates an empty manager presenting `local_identity` to remote
    /// platforms.
    pub fn new(local_identity: impl Into<String>) -> Self {
        Self {
            local_identity: local_identity.into(),
            upstreams: BTreeMap::new(),
            downstreams: BTreeMap::new(),
            control_owner: BTreeMap::new(),
        }
    }

    /// Number of registered upstream links.
    pub fn upstream_count(&self) -> usize {
        self.upstreams.len()
    }

    /// Number of enrolled downstream platforms.
    pub fn downstream_count(&self) -> usize {
        self.downstreams.len()
    }

    /// Maps a persistent [`GbPlatformLink`] aggregate to a [`CascadeConfig`].
    ///
    /// Backoff, subscription and interval policy are taken from the aggregate
    /// so the durable control-plane record is the single source of truth.
    pub fn map_config(
        local_identity: &str,
        link: &GbPlatformLink,
    ) -> Result<CascadeConfig, CascadeRoutingError> {
        let endpoint = link.endpoint();
        let domain_id = DomainId::new(endpoint.domain.clone()).ok_or_else(|| {
            CascadeRoutingError::Cascade(CascadeError::internal("invalid domain id".to_string()))
        })?;
        let local_uri = parse_uri(&format!("sip:{}@{}", local_identity, endpoint.domain))?;
        let upstream_uri = parse_uri(&format!(
            "sip:{}@{}:{}",
            link.identity().remote.as_str(),
            endpoint.host,
            endpoint.port
        ))?;

        let mut config = CascadeConfig::with_options(
            domain_id,
            local_uri,
            upstream_uri,
            endpoint.realm.clone(),
            link.credential().credential_ref.clone(),
            link.register_interval_secs(),
            30,
            link.credential().allow_md5,
            false,
        )
        .map_err(CascadeRoutingError::Cascade)?;

        let backoff = link.backoff();
        config.base_backoff_ms = backoff.base_ms;
        config.max_backoff_ms = backoff.max_ms;
        config.jitter_ms = backoff.jitter_ms;
        config.max_retries = backoff.max_retries;
        config.subscription_max_subscriptions = link.subscription_limits().max_subscriptions;
        Ok(config)
    }

    /// Registers an upstream link built from `link`, using `provider` for
    /// credential resolution.
    ///
    /// Rejects downstream links, duplicate remote identities and any remote
    /// identity equal to the local platform identity (a self-loop).
    pub fn add_upstream(
        &mut self,
        link: &GbPlatformLink,
        provider: P,
    ) -> Result<(), CascadeRoutingError> {
        if link.direction() != PlatformDirection::Upstream {
            return Err(CascadeRoutingError::WrongDirection {
                expected: "upstream",
            });
        }
        let remote_identity = link.identity().remote.as_str().to_string();
        self.validate_distinct_identity(&remote_identity)?;
        if self
            .upstreams
            .values()
            .any(|u| u.remote_identity == remote_identity)
        {
            return Err(CascadeRoutingError::DuplicateRemote);
        }

        let config = Self::map_config(&self.local_identity, link)?;
        let machine =
            Gb28181Cascade::new(config, provider).map_err(CascadeRoutingError::Cascade)?;
        self.upstreams.insert(
            link.platform_link_id(),
            ManagedUpstream {
                remote_identity,
                acl: link.acl().clone(),
                machine,
            },
        );
        Ok(())
    }

    /// Enrolls a downstream platform, validating direction and identity.
    pub fn enroll_downstream(&mut self, link: &GbPlatformLink) -> Result<(), CascadeRoutingError> {
        if link.direction() != PlatformDirection::Downstream {
            return Err(CascadeRoutingError::WrongDirection {
                expected: "downstream",
            });
        }
        let remote_identity = link.identity().remote.as_str().to_string();
        self.validate_distinct_identity(&remote_identity)?;
        if self.downstreams.contains_key(&remote_identity) {
            return Err(CascadeRoutingError::DuplicateRemote);
        }
        self.downstreams.insert(
            remote_identity,
            DownstreamEnrollment {
                link_id: link.platform_link_id(),
                acl: link.acl().clone(),
            },
        );
        Ok(())
    }

    /// Removes an upstream link and releases any control it owned.
    pub fn remove_upstream(&mut self, link_id: PlatformLinkId) {
        self.upstreams.remove(&link_id);
        self.control_owner.retain(|_, owner| *owner != link_id);
    }

    /// Drives a single upstream link. Errors are scoped to that link and never
    /// affect the other links held by the manager.
    pub fn process(
        &mut self,
        link_id: PlatformLinkId,
        input: CascadeInput,
    ) -> Result<Vec<CascadeOutput>, CascadeRoutingError> {
        let link = self
            .upstreams
            .get_mut(&link_id)
            .ok_or(CascadeRoutingError::UnknownLink)?;
        link.machine
            .process(input)
            .map_err(CascadeRoutingError::Cascade)
    }

    /// Ticks every upstream link independently, collecting the per-link result.
    ///
    /// A failing link yields an `Err` entry but does not prevent the remaining
    /// links from advancing.
    pub fn tick_all(
        &mut self,
        now: u64,
    ) -> Vec<(PlatformLinkId, Result<Vec<CascadeOutput>, CascadeError>)> {
        self.upstreams
            .iter_mut()
            .map(|(id, link)| {
                let result = link.machine.process(CascadeInput {
                    now,
                    event: CascadeEvent::Tick,
                });
                (*id, result)
            })
            .collect()
    }

    /// Returns `true` when the link may share the catalog resource `external_id`.
    pub fn may_share_catalog(
        &self,
        link_id: PlatformLinkId,
        external_id: &str,
    ) -> Result<bool, CascadeRoutingError> {
        Ok(self.acl_for(link_id)?.allows_resource(external_id))
    }

    /// Returns `true` when the link may issue control commands.
    pub fn may_control(&self, link_id: PlatformLinkId) -> Result<bool, CascadeRoutingError> {
        Ok(self.acl_for(link_id)?.allow_control)
    }

    /// Returns `true` when the link may negotiate a media bridge.
    pub fn may_bridge(&self, link_id: PlatformLinkId) -> Result<bool, CascadeRoutingError> {
        Ok(self.acl_for(link_id)?.allow_media)
    }

    /// Validates that a bridge to `link_id` extending the already-visited
    /// `via_path` is safe: it must not revisit a platform, exceed the hop
    /// limit, target a denied platform, or violate the media ACL.
    pub fn authorize_bridge(
        &self,
        link_id: PlatformLinkId,
        via_path: &[&str],
    ) -> Result<(), CascadeRoutingError> {
        let link = self
            .upstreams
            .get(&link_id)
            .ok_or(CascadeRoutingError::UnknownLink)?;
        if !link.acl.allow_media {
            return Err(CascadeRoutingError::AclDenied);
        }
        if via_path.len() >= MAX_CASCADE_HOPS {
            return Err(CascadeRoutingError::HopLimitExceeded);
        }
        if link.acl.is_denied_platform(&link.remote_identity) {
            return Err(CascadeRoutingError::LoopDetected);
        }
        let path: Vec<String> = via_path.iter().map(|h| (*h).to_string()).collect();
        if detect_loop(&path, &link.remote_identity) || detect_loop(&path, &self.local_identity) {
            return Err(CascadeRoutingError::LoopDetected);
        }
        Ok(())
    }

    /// Acquires exclusive control of `resource_external_id` for `link_id`.
    ///
    /// Idempotent for the current owner; rejects a different link with
    /// [`CascadeRoutingError::ControlConflict`].
    pub fn acquire_control(
        &mut self,
        resource_external_id: &str,
        link_id: PlatformLinkId,
    ) -> Result<(), CascadeRoutingError> {
        if !self.upstreams.contains_key(&link_id) {
            return Err(CascadeRoutingError::UnknownLink);
        }
        match self.control_owner.get(resource_external_id) {
            Some(owner) if *owner == link_id => Ok(()),
            Some(_) => Err(CascadeRoutingError::ControlConflict),
            None => {
                self.control_owner
                    .insert(resource_external_id.to_string(), link_id);
                Ok(())
            }
        }
    }

    /// Releases control of `resource_external_id` held by `link_id`.
    pub fn release_control(&mut self, resource_external_id: &str, link_id: PlatformLinkId) {
        if self.control_owner.get(resource_external_id) == Some(&link_id) {
            self.control_owner.remove(resource_external_id);
        }
    }

    /// Confirms that an inbound request presenting `platform_identity` matches
    /// an enrolled upstream or downstream *platform* (never a device identity).
    pub fn validate_platform_identity(
        &self,
        platform_identity: &str,
    ) -> Result<PlatformLinkId, CascadeRoutingError> {
        if let Some((id, _)) = self
            .upstreams
            .iter()
            .find(|(_, u)| u.remote_identity == platform_identity)
        {
            return Ok(*id);
        }
        if let Some(enrollment) = self.downstreams.get(platform_identity) {
            return Ok(enrollment.link_id);
        }
        Err(CascadeRoutingError::IdentityMismatch)
    }

    fn acl_for(&self, link_id: PlatformLinkId) -> Result<&PlatformAcl, CascadeRoutingError> {
        self.upstreams
            .get(&link_id)
            .map(|u| &u.acl)
            .or_else(|| {
                self.downstreams
                    .values()
                    .find(|d| d.link_id == link_id)
                    .map(|d| &d.acl)
            })
            .ok_or(CascadeRoutingError::UnknownLink)
    }

    fn validate_distinct_identity(&self, remote: &str) -> Result<(), CascadeRoutingError> {
        if remote == self.local_identity {
            return Err(CascadeRoutingError::IdentityMismatch);
        }
        Ok(())
    }
}

fn parse_uri(raw: &str) -> Result<SipUri, CascadeRoutingError> {
    SipUri::parse(raw).map_err(|e| CascadeRoutingError::Cascade(CascadeError::from(e)))
}

#[cfg(test)]
mod tests;

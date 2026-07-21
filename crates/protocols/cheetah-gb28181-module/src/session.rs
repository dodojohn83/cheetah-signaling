//! Persistent GB28181 session transaction link.
//!
//! This module owns the REGISTER / unregister / refresh / keepalive / expiry /
//! owner-acquisition *business* transitions and drives them against the
//! [`ProtocolSessionRepository`] port. It is the seam that makes the persistent
//! [`ProtocolSession`] aggregate authoritative for a device's registration
//! binding (`GB4-ACC-002`).
//!
//! # Layering
//!
//! The protocol driver only maps SIP messages to `AccessInput` and executes
//! `AccessOutput`; it never touches storage. Identity/ownership resolution
//! (tenant, device, owner node and epoch) is performed by the application,
//! which hands a [`SessionContext`] to this link. The link then applies the
//! aggregate transitions and persists them through the port. No SQL, NATS, or
//! concrete storage type is referenced here, so the module stays within layer
//! four.
//!
//! # Concurrency and fencing
//!
//! Every write goes through the aggregate's optimistic-concurrency [`Revision`]
//! and each transition bumps the revision exactly once between saves. Owner
//! epochs fence stale shards: a transition carrying an epoch older than the
//! stored owner epoch is rejected, and ownership acquisition must strictly
//! increase the epoch.

use std::sync::Arc;

use cheetah_domain::{
    CompatibilityProfile, DomainError, LocalIdentity, NewProtocolSession, PresenceState, Protocol,
    ProtocolSession, ProtocolSessionRepository, RegistrationInfo, SessionEndpoint, SipTransport,
};
use cheetah_signal_types::{
    Clock, DeviceId, IdGenerator, MAX_PAGE_SIZE, NodeId, OwnerEpoch, PageRequest, ProtocolIdentity,
    ProtocolSessionId, Revision, TenantId, UtcTimestamp,
};

/// Reason recorded when the expiry reaper marks a session offline.
const REASON_EXPIRED: &str = "expired";

/// Errors returned by the session transaction link.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionLinkError {
    /// A keepalive or unregister referenced a device with no active session.
    #[error("no active protocol session for the device")]
    NotRegistered,
    /// A keepalive arrived for a session whose registration already expired.
    ///
    /// The device must re-REGISTER rather than being silently revived.
    #[error("protocol session expired; re-registration required")]
    Expired,
    /// The transition carried an owner epoch older than the current owner.
    #[error("stale owner epoch: current {current}, got {got}")]
    StaleOwner {
        /// Epoch currently fencing the session.
        current: u64,
        /// Epoch carried by the rejected transition.
        got: u64,
    },
    /// The repository port failed (including optimistic-concurrency conflicts).
    #[error(transparent)]
    Repository(#[from] DomainError),
}

/// Identity and ownership context resolved by the application for a device.
///
/// The application resolves the tenant and internal [`DeviceId`] from listener
/// routing and the GB device id, and supplies the owner node and epoch held by
/// the shard that terminated the transaction. The link treats these as trusted
/// inputs and does not itself perform routing.
#[derive(Clone, Debug)]
pub struct SessionContext {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Internal device identifier.
    pub device_id: DeviceId,
    /// External GB28181 device identity.
    pub protocol_identity: ProtocolIdentity,
    /// Local listener identity that terminated the registration.
    pub local_identity: LocalIdentity,
    /// SIP transport carrying the registration.
    pub transport: SipTransport,
    /// Node currently owning the device's session.
    pub owner_node_id: NodeId,
    /// Owner epoch fencing the session.
    pub owner_epoch: OwnerEpoch,
    /// Compatibility profile applied to the device.
    pub compatibility: CompatibilityProfile,
}

/// SIP transaction facts for an authenticated REGISTER.
#[derive(Clone, Debug)]
pub struct RegisterParams {
    /// Endpoint routing facts derived from Via / Contact / source.
    pub endpoint: SessionEndpoint,
    /// REGISTER transaction facts (Call-ID, CSeq, Expires).
    pub registration: RegistrationInfo,
    /// Absolute time at which the registration expires.
    pub expiry_at: UtcTimestamp,
}

/// Outcome of an authenticated REGISTER.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterOutcome {
    /// A new session was created.
    Created {
        /// Identifier of the created session.
        protocol_session_id: ProtocolSessionId,
        /// Owner epoch assigned to the session.
        owner_epoch: OwnerEpoch,
    },
    /// An existing session was refreshed.
    Refreshed {
        /// Identifier of the refreshed session.
        protocol_session_id: ProtocolSessionId,
        /// Revision after the refresh.
        revision: Revision,
    },
}

/// Persists GB28181 registration/session transitions through the repository
/// port.
///
/// The link is cheap to clone and holds no per-device state; all authoritative
/// state lives in the [`ProtocolSession`] aggregate behind the repository.
#[derive(Clone)]
pub struct ProtocolSessionLink {
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
}

impl std::fmt::Debug for ProtocolSessionLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolSessionLink")
            .finish_non_exhaustive()
    }
}

impl ProtocolSessionLink {
    /// Creates a new link over the injected clock and id generator.
    pub fn new(clock: Arc<dyn Clock>, id_generator: Arc<dyn IdGenerator>) -> Self {
        Self {
            clock,
            id_generator,
        }
    }

    /// Applies an authenticated REGISTER: creates a session when none exists,
    /// otherwise refreshes the binding (expiry, endpoint, CSeq) and bumps the
    /// revision. Ownership takeover observed during a REGISTER is applied before
    /// the refresh so the fencing epoch stays monotonic.
    pub async fn register(
        &self,
        repo: &mut dyn ProtocolSessionRepository,
        ctx: &SessionContext,
        params: RegisterParams,
    ) -> Result<RegisterOutcome, SessionLinkError> {
        let existing = repo
            .get_by_device(ctx.tenant_id, Protocol::Gb28181, ctx.device_id)
            .await?;

        match existing {
            None => {
                let session = ProtocolSession::new(
                    self.clock.as_ref(),
                    NewProtocolSession {
                        protocol_session_id: self.id_generator.generate_protocol_session_id(),
                        tenant_id: ctx.tenant_id,
                        device_id: ctx.device_id,
                        protocol: Protocol::Gb28181,
                        protocol_identity: ctx.protocol_identity.clone(),
                        local_identity: ctx.local_identity.clone(),
                        transport: ctx.transport,
                        endpoint: params.endpoint,
                        registration: params.registration,
                        expiry_at: params.expiry_at,
                        owner_node_id: Some(ctx.owner_node_id),
                        owner_epoch: ctx.owner_epoch,
                        compatibility: ctx.compatibility.clone(),
                    },
                )?;
                let protocol_session_id = session.protocol_session_id();
                repo.save(&session).await?;
                Ok(RegisterOutcome::Created {
                    protocol_session_id,
                    owner_epoch: ctx.owner_epoch,
                })
            }
            Some(mut session) => {
                self.ensure_owner_current(&session, ctx.owner_epoch)?;
                if session.owner_node_id() != Some(ctx.owner_node_id)
                    || ctx.owner_epoch > session.owner_epoch()
                {
                    session.assign_owner(self.clock.as_ref(), ctx.owner_node_id, ctx.owner_epoch);
                    repo.save(&session).await?;
                }
                session.refresh_registration(
                    self.clock.as_ref(),
                    params.registration,
                    params.expiry_at,
                    Some(params.endpoint),
                )?;
                let protocol_session_id = session.protocol_session_id();
                let revision = session.revision();
                repo.save(&session).await?;
                Ok(RegisterOutcome::Refreshed {
                    protocol_session_id,
                    revision,
                })
            }
        }
    }

    /// Applies an explicit unregister (`Expires=0`): removes the registration
    /// binding for the device. Returns the deleted session id, or `None` when
    /// no binding existed (idempotent). A later keepalive without an active
    /// session is rejected, forcing the device to re-REGISTER.
    pub async fn unregister(
        &self,
        repo: &mut dyn ProtocolSessionRepository,
        ctx: &SessionContext,
    ) -> Result<Option<ProtocolSessionId>, SessionLinkError> {
        let Some(session) = repo
            .get_by_device(ctx.tenant_id, Protocol::Gb28181, ctx.device_id)
            .await?
        else {
            return Ok(None);
        };
        self.ensure_owner_current(&session, ctx.owner_epoch)?;
        let protocol_session_id = session.protocol_session_id();
        repo.delete(ctx.tenant_id, protocol_session_id, session.revision())
            .await?;
        Ok(Some(protocol_session_id))
    }

    /// Records a keepalive: updates `last_keepalive_at`, keeps the device
    /// online, and bumps the revision. Rejects a keepalive with no active
    /// session, an expired session, or a stale owner epoch.
    pub async fn keepalive(
        &self,
        repo: &mut dyn ProtocolSessionRepository,
        ctx: &SessionContext,
    ) -> Result<(), SessionLinkError> {
        let now = self.clock.now_wall();
        let Some(mut session) = repo
            .get_by_device(ctx.tenant_id, Protocol::Gb28181, ctx.device_id)
            .await?
        else {
            return Err(SessionLinkError::NotRegistered);
        };
        self.ensure_owner_current(&session, ctx.owner_epoch)?;
        if session.is_expired(now) {
            return Err(SessionLinkError::Expired);
        }
        session.record_keepalive(self.clock.as_ref());
        repo.save(&session).await?;
        Ok(())
    }

    /// Acquires ownership of a device's session for `node_id` at `new_epoch`.
    ///
    /// The epoch must strictly increase; an equal or older epoch is rejected as
    /// stale. Returns the new revision, or `None` when the device has no
    /// session to take over yet.
    pub async fn acquire_owner(
        &self,
        repo: &mut dyn ProtocolSessionRepository,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        new_epoch: OwnerEpoch,
    ) -> Result<Option<Revision>, SessionLinkError> {
        let Some(mut session) = repo
            .get_by_device(tenant_id, Protocol::Gb28181, device_id)
            .await?
        else {
            return Ok(None);
        };
        if new_epoch.0 <= session.owner_epoch().0 {
            return Err(SessionLinkError::StaleOwner {
                current: session.owner_epoch().0,
                got: new_epoch.0,
            });
        }
        session.assign_owner(self.clock.as_ref(), node_id, new_epoch);
        let revision = session.revision();
        repo.save(&session).await?;
        Ok(Some(revision))
    }

    /// Marks offline every session whose `expiry_at` has passed at `now`.
    ///
    /// Intended to be called from a `Runtime` tick/reaper. The sweep reads
    /// expired sessions in bounded pages (at most `max_sessions`), then marks
    /// each still-online session offline. It is idempotent: already-offline
    /// sessions are skipped, and a concurrent modification on one session is
    /// skipped rather than aborting the whole sweep. Returns the number of
    /// sessions transitioned to offline.
    ///
    /// `page_size` is clamped to `[1, MAX_PAGE_SIZE]` so an out-of-range
    /// configuration cannot disable the sweep.
    pub async fn reap_expired(
        &self,
        repo: &mut dyn ProtocolSessionRepository,
        now: UtcTimestamp,
        page_size: u32,
        max_sessions: usize,
    ) -> Result<usize, SessionLinkError> {
        let page_size = page_size.clamp(1, MAX_PAGE_SIZE);
        let mut expired: Vec<ProtocolSession> = Vec::new();
        let mut cursor: Option<String> = None;
        while expired.len() < max_sessions {
            let mut page = PageRequest::new(page_size)
                .map_err(|e| DomainError::invalid_argument(e.to_string()))?;
            page.cursor = cursor;
            let result = repo.list_expired(now, page).await?;
            for session in result.items {
                if expired.len() >= max_sessions {
                    break;
                }
                expired.push(session);
            }
            match result.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        let mut reaped = 0;
        for mut session in expired {
            if session.presence() == PresenceState::Offline {
                continue;
            }
            session.mark_offline(self.clock.as_ref(), REASON_EXPIRED);
            match repo.save(&session).await {
                Ok(()) => reaped += 1,
                Err(DomainError::ConcurrentModification { .. }) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(reaped)
    }

    /// Rejects a transition whose owner epoch is older than the stored one.
    fn ensure_owner_current(
        &self,
        session: &ProtocolSession,
        got: OwnerEpoch,
    ) -> Result<(), SessionLinkError> {
        if got.0 < session.owner_epoch().0 {
            return Err(SessionLinkError::StaleOwner {
                current: session.owner_epoch().0,
                got: got.0,
            });
        }
        Ok(())
    }
}

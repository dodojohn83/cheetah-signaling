//! Portable device actor API and actor context.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use async_trait::async_trait;

use cheetah_signal_types::{Clock, DeviceId, DurationMs, TenantId};

use crate::{DeviceKey, RuntimeError, RuntimeMessage, Scheduler, SessionRegistry, TimerId};

/// Portable Sans-I/O device actor.
///
/// A concrete implementation is provided by a protocol module and executed by a
/// `Runtime` without spawning a per-device Tokio task.
#[async_trait]
pub trait DeviceActor: Send + Sized + 'static {
    /// Handle type stored in the session registry.
    type SessionHandle: Clone + Send + Sync + 'static;

    /// Output type produced by the actor.
    type Output: Send + 'static;

    /// Error type returned by the actor.
    type Error: std::fmt::Display + From<RuntimeError> + Send + 'static;

    /// Creates the actor for a device.
    fn create(ctx: ActorContext<Self::SessionHandle>) -> Result<Self, Self::Error>;

    /// Handles one runtime message.
    async fn handle(
        &mut self,
        message: RuntimeMessage,
        ctx: &ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error>;

    /// Shuts down the actor.
    async fn shutdown(
        self,
        ctx: &ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error>;
}

/// Context passed to a `DeviceActor`.
///
/// The context is cheap to clone and carries the scheduler, clock, session
/// registry, actor-specific configuration, and per-device identity needed by
/// the actor.
#[derive(Clone)]
pub struct ActorContext<Handle: Clone + Send + Sync + 'static> {
    device_key: DeviceKey,
    scheduler: Arc<dyn Scheduler>,
    clock: Arc<dyn Clock>,
    id_generator: Arc<AtomicU64>,
    session_registry: SessionRegistry<Handle>,
    actor_config: Option<Arc<dyn Any + Send + Sync>>,
}

impl<Handle: Clone + Send + Sync + 'static> std::fmt::Debug for ActorContext<Handle> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorContext")
            .field("device_key", &self.device_key)
            .field("actor_config", &self.actor_config.is_some())
            .finish_non_exhaustive()
    }
}

impl<Handle: Clone + Send + Sync + 'static> ActorContext<Handle> {
    /// Creates a new actor context.
    pub fn new(
        device_key: DeviceKey,
        scheduler: Arc<dyn Scheduler>,
        clock: Arc<dyn Clock>,
        id_generator: Arc<AtomicU64>,
        session_registry: SessionRegistry<Handle>,
        actor_config: Option<Arc<dyn Any + Send + Sync>>,
    ) -> Self {
        Self {
            device_key,
            scheduler,
            clock,
            id_generator,
            session_registry,
            actor_config,
        }
    }

    /// Returns the device key for this actor.
    pub fn device_key(&self) -> DeviceKey {
        self.device_key
    }

    /// Returns the tenant identifier for this actor.
    pub fn tenant_id(&self) -> TenantId {
        self.device_key.tenant_id()
    }

    /// Returns the device identifier for this actor.
    pub fn device_id(&self) -> DeviceId {
        self.device_key.device_id()
    }

    /// Returns the session registry.
    pub fn session_registry(&self) -> &SessionRegistry<Handle> {
        &self.session_registry
    }

    /// Returns the clock.
    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// Returns the actor-specific configuration, if any.
    pub fn actor_config<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.actor_config
            .as_ref()
            .and_then(|cfg| cfg.clone().downcast::<T>().ok())
    }

    /// Schedules a timer and returns its identifier.
    pub async fn schedule_timer<S: Into<String>>(
        &self,
        delay: DurationMs,
        kind: S,
    ) -> Result<TimerId, RuntimeError> {
        let timer_id = TimerId::new(
            self.id_generator
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        );
        self.scheduler
            .schedule(self.device_key, timer_id, delay, kind.into())
            .await?;
        Ok(timer_id)
    }

    /// Cancels a previously scheduled timer.
    pub async fn cancel_timer(&self, timer_id: TimerId) -> Result<(), RuntimeError> {
        self.scheduler.cancel(self.device_key, timer_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheetah_signal_types::DurationMs;

    #[test]
    fn context_exposes_identity() {
        let tenant_id = cheetah_signal_types::TenantId::generate();
        let device_id = cheetah_signal_types::DeviceId::generate();
        let key = DeviceKey::new(tenant_id, device_id);
        let ctx = ActorContext::<String>::new(
            key,
            Arc::new(NoopScheduler),
            Arc::new(NoopClock),
            Arc::new(AtomicU64::new(1)),
            SessionRegistry::new(10),
            None,
        );
        assert_eq!(ctx.device_key(), key);
        assert_eq!(ctx.tenant_id(), tenant_id);
        assert_eq!(ctx.device_id(), device_id);
    }

    struct NoopScheduler;
    #[async_trait]
    impl Scheduler for NoopScheduler {
        async fn schedule(
            &self,
            _device_key: DeviceKey,
            _timer_id: TimerId,
            _delay: DurationMs,
            _kind: String,
        ) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn cancel(
            &self,
            _device_key: DeviceKey,
            _timer_id: TimerId,
        ) -> Result<(), RuntimeError> {
            Ok(())
        }
    }

    struct NoopClock;
    impl Clock for NoopClock {
        fn now_wall(&self) -> cheetah_signal_types::UtcTimestamp {
            cheetah_signal_types::UtcTimestamp::default()
        }

        fn now_monotonic(&self) -> DurationMs {
            DurationMs::default()
        }
    }
}

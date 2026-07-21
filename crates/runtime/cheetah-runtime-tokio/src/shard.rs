//! Fixed shard worker that processes device messages sequentially.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use cheetah_runtime_api::{
    ActorContext, DeviceActor, DeviceKey, RuntimeConfig, RuntimeMessage, RuntimeMetrics, Scheduler,
    SessionRegistry,
};
use cheetah_signal_types::Clock;
use tokio::sync::mpsc;

/// Per-shard configuration derived from the runtime configuration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShardConfig {
    pub(crate) max_messages_per_poll: usize,
    pub(crate) max_consecutive_per_device: usize,
    pub(crate) actor_idle_timeout_ms: u64,
}

impl From<&RuntimeConfig> for ShardConfig {
    fn from(config: &RuntimeConfig) -> Self {
        Self {
            max_messages_per_poll: config.max_messages_per_poll,
            max_consecutive_per_device: config.max_consecutive_per_device,
            actor_idle_timeout_ms: config.actor_idle_timeout_ms,
        }
    }
}

/// Shared context for a single shard worker.
pub(crate) struct ShardContext<A: DeviceActor> {
    config: ShardConfig,
    output_tx: mpsc::Sender<A::Output>,
    scheduler: Arc<dyn Scheduler>,
    clock: Arc<dyn Clock>,
    id_gen: Arc<AtomicU64>,
    session_registry: SessionRegistry<A::SessionHandle>,
    metrics: Arc<RuntimeMetrics>,
}

impl<A: DeviceActor> ShardContext<A> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        config: ShardConfig,
        output_tx: mpsc::Sender<A::Output>,
        scheduler: Arc<dyn Scheduler>,
        clock: Arc<dyn Clock>,
        id_gen: Arc<AtomicU64>,
        session_registry: SessionRegistry<A::SessionHandle>,
        metrics: Arc<RuntimeMetrics>,
    ) -> Self {
        Self {
            config,
            output_tx,
            scheduler,
            clock,
            id_gen,
            session_registry,
            metrics,
        }
    }

    fn actor_context(&self, device_key: DeviceKey) -> ActorContext<A::SessionHandle> {
        ActorContext::new(
            device_key,
            self.scheduler.clone(),
            self.clock.clone(),
            self.id_gen.clone(),
            self.session_registry.clone(),
        )
    }
}

/// A fixed shard worker.
pub(crate) struct Shard;

impl Shard {
    /// Starts a shard task and returns its future.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run<A: DeviceActor>(
        config: ShardConfig,
        mut receiver: mpsc::Receiver<RuntimeMessage>,
        output_tx: mpsc::Sender<A::Output>,
        scheduler: Arc<dyn Scheduler>,
        clock: Arc<dyn Clock>,
        id_gen: Arc<AtomicU64>,
        session_registry: SessionRegistry<A::SessionHandle>,
        metrics: Arc<RuntimeMetrics>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(async move {
            let mut ready_queue: BTreeMap<DeviceKey, VecDeque<RuntimeMessage>> = BTreeMap::new();
            let mut actors: BTreeMap<DeviceKey, A> = BTreeMap::new();
            let mut last_active: BTreeMap<DeviceKey, i64> = BTreeMap::new();
            let ctx = ShardContext::new(
                config,
                output_tx,
                scheduler,
                clock,
                id_gen,
                session_registry,
                metrics,
            );

            let mut shutdown = false;

            loop {
                while !ready_queue.is_empty() {
                    process_ready_queue(&mut ready_queue, &mut actors, &mut last_active, &ctx)
                        .await;
                }

                evict_idle(&mut actors, &mut last_active, &ctx);

                if shutdown {
                    shutdown_all(actors, &ctx).await;
                    break;
                }

                match receiver.recv().await {
                    Some(msg) => {
                        if matches!(&msg, RuntimeMessage::Shutdown) {
                            shutdown = true;
                        } else if let Some(device_key) = msg.device_key() {
                            ready_queue.entry(device_key).or_default().push_back(msg);
                        }
                        while let Ok(next) = receiver.try_recv() {
                            if matches!(&next, RuntimeMessage::Shutdown) {
                                shutdown = true;
                            } else if let Some(device_key) = next.device_key() {
                                ready_queue.entry(device_key).or_default().push_back(next);
                            }
                        }
                    }
                    None => {
                        shutdown = true;
                    }
                }
            }
        })
    }
}

async fn process_ready_queue<A: DeviceActor>(
    ready_queue: &mut BTreeMap<DeviceKey, VecDeque<RuntimeMessage>>,
    actors: &mut BTreeMap<DeviceKey, A>,
    last_active: &mut BTreeMap<DeviceKey, i64>,
    ctx: &ShardContext<A>,
) {
    let max = ctx.config.max_messages_per_poll;
    let mut processed = 0usize;

    while !ready_queue.is_empty() && processed < max {
        let keys: Vec<DeviceKey> = ready_queue.keys().copied().collect();

        for key in keys {
            if processed >= max {
                break;
            }

            let actor_ctx = ctx.actor_context(key);

            let actor = if let Some(existing) = actors.get_mut(&key) {
                existing
            } else {
                match A::create(actor_ctx.clone()) {
                    Ok(actor) => {
                        actors.insert(key, actor);
                        ctx.metrics.record_actor_created();
                        if let Some(new) = actors.get_mut(&key) {
                            new
                        } else {
                            continue;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(?key, "failed to create actor: {e}");
                        ready_queue.remove(&key);
                        continue;
                    }
                }
            };

            let queue = match ready_queue.get_mut(&key) {
                Some(queue) => queue,
                None => continue,
            };

            for _ in 0..ctx.config.max_consecutive_per_device {
                if processed >= max {
                    break;
                }

                let msg = match queue.pop_front() {
                    Some(msg) => msg,
                    None => break,
                };

                match actor.handle(msg, &actor_ctx).await {
                    Ok(outputs) => {
                        for output in outputs {
                            if ctx.output_tx.send(output).await.is_err() {
                                tracing::warn!("output channel closed");
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(?key, "actor handle error: {e}");
                    }
                }

                processed += 1;
                ctx.metrics.record_message_processed();
                last_active.insert(key, ctx.clock.now_monotonic().as_millis());
            }

            let queue_is_empty = queue.is_empty();
            if queue_is_empty {
                ready_queue.remove(&key);
            }
        }
    }
}

/// Lazily unloads actors whose in-memory state has been idle beyond the
/// configured timeout. Authoritative state lives in repositories/Operations, so
/// an unloaded actor is transparently recreated on its next message.
fn evict_idle<A: DeviceActor>(
    actors: &mut BTreeMap<DeviceKey, A>,
    last_active: &mut BTreeMap<DeviceKey, i64>,
    ctx: &ShardContext<A>,
) {
    let timeout = ctx.config.actor_idle_timeout_ms;
    if timeout == 0 {
        return;
    }
    let now = ctx.clock.now_monotonic().as_millis();
    let timeout = timeout as i64;
    let stale: Vec<DeviceKey> = last_active
        .iter()
        .filter(|&(_, &seen)| now.saturating_sub(seen) >= timeout)
        .map(|(key, _)| *key)
        .collect();
    for key in stale {
        last_active.remove(&key);
        if actors.remove(&key).is_some() {
            ctx.metrics.record_actor_evicted_idle();
        }
    }
}

async fn shutdown_all<A: DeviceActor>(actors: BTreeMap<DeviceKey, A>, ctx: &ShardContext<A>) {
    for (key, actor) in actors {
        ctx.metrics.decrement_active_actors();
        let actor_ctx = ctx.actor_context(key);
        match actor.shutdown(&actor_ctx).await {
            Ok(outputs) => {
                for output in outputs {
                    if ctx.output_tx.send(output).await.is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(?key, "actor shutdown error: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    use async_trait::async_trait;
    use cheetah_runtime_api::{ActorContext, RuntimeError, TimerId};
    use cheetah_signal_types::{DeviceId, DurationMs, TenantId, UtcTimestamp};

    struct TestActor;

    #[async_trait]
    impl DeviceActor for TestActor {
        type SessionHandle = String;
        type Output = String;
        type Error = RuntimeError;

        fn create(_ctx: ActorContext<Self::SessionHandle>) -> Result<Self, Self::Error> {
            Ok(Self)
        }

        async fn handle(
            &mut self,
            _message: RuntimeMessage,
            _ctx: &ActorContext<Self::SessionHandle>,
        ) -> Result<Vec<Self::Output>, Self::Error> {
            Ok(vec![])
        }

        async fn shutdown(
            self,
            _ctx: &ActorContext<Self::SessionHandle>,
        ) -> Result<Vec<Self::Output>, Self::Error> {
            Ok(vec![])
        }
    }

    #[derive(Default)]
    struct TestClock {
        now_ms: AtomicI64,
    }

    impl TestClock {
        fn set(&self, ms: i64) {
            self.now_ms.store(ms, Ordering::SeqCst);
        }
    }

    impl Clock for TestClock {
        fn now_wall(&self) -> UtcTimestamp {
            UtcTimestamp::default()
        }

        fn now_monotonic(&self) -> DurationMs {
            DurationMs::from_millis(self.now_ms.load(Ordering::SeqCst))
        }
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

    fn context(clock: Arc<TestClock>, idle_timeout_ms: u64) -> ShardContext<TestActor> {
        let (output_tx, _output_rx) = mpsc::channel(4);
        ShardContext::new(
            ShardConfig {
                max_messages_per_poll: 16,
                max_consecutive_per_device: 4,
                actor_idle_timeout_ms: idle_timeout_ms,
            },
            output_tx,
            Arc::new(NoopScheduler),
            clock,
            Arc::new(AtomicU64::new(1)),
            SessionRegistry::new(16),
            Arc::new(RuntimeMetrics::new()),
        )
    }

    fn device_key() -> DeviceKey {
        DeviceKey::new(TenantId::generate(), DeviceId::generate())
    }

    #[test]
    fn evict_idle_unloads_stale_actors_only() {
        let clock = Arc::new(TestClock::default());
        let ctx = context(clock.clone(), 1000);
        let stale = device_key();
        let fresh = device_key();

        let mut actors: BTreeMap<DeviceKey, TestActor> = BTreeMap::new();
        actors.insert(stale, TestActor);
        actors.insert(fresh, TestActor);
        ctx.metrics.record_actor_created();
        ctx.metrics.record_actor_created();

        let mut last_active: BTreeMap<DeviceKey, i64> = BTreeMap::new();
        last_active.insert(stale, 0);
        last_active.insert(fresh, 500);

        clock.set(1200);
        evict_idle(&mut actors, &mut last_active, &ctx);

        assert!(!actors.contains_key(&stale));
        assert!(actors.contains_key(&fresh));
        assert!(!last_active.contains_key(&stale));
        let snapshot = ctx.metrics.snapshot();
        assert_eq!(snapshot.actors_evicted_idle, 1);
        assert_eq!(snapshot.active_actors, 1);
    }

    #[test]
    fn evict_idle_disabled_when_timeout_zero() {
        let clock = Arc::new(TestClock::default());
        let ctx = context(clock.clone(), 0);
        let key = device_key();

        let mut actors: BTreeMap<DeviceKey, TestActor> = BTreeMap::new();
        actors.insert(key, TestActor);
        let mut last_active: BTreeMap<DeviceKey, i64> = BTreeMap::new();
        last_active.insert(key, 0);

        clock.set(10_000_000);
        evict_idle(&mut actors, &mut last_active, &ctx);

        assert!(actors.contains_key(&key));
    }
}

//! Fixed shard worker that processes device messages sequentially.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use cheetah_runtime_api::{
    ActorContext, DeviceActor, DeviceKey, RuntimeConfig, RuntimeMessage, Scheduler, SessionRegistry,
};
use cheetah_signal_types::Clock;
use tokio::sync::mpsc;

/// Per-shard configuration derived from the runtime configuration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShardConfig {
    pub(crate) max_messages_per_poll: usize,
    pub(crate) max_consecutive_per_device: usize,
}

impl From<&RuntimeConfig> for ShardConfig {
    fn from(config: &RuntimeConfig) -> Self {
        Self {
            max_messages_per_poll: config.max_messages_per_poll,
            max_consecutive_per_device: config.max_consecutive_per_device,
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
}

impl<A: DeviceActor> ShardContext<A> {
    fn new(
        config: ShardConfig,
        output_tx: mpsc::Sender<A::Output>,
        scheduler: Arc<dyn Scheduler>,
        clock: Arc<dyn Clock>,
        id_gen: Arc<AtomicU64>,
        session_registry: SessionRegistry<A::SessionHandle>,
    ) -> Self {
        Self {
            config,
            output_tx,
            scheduler,
            clock,
            id_gen,
            session_registry,
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
    pub(crate) fn run<A: DeviceActor>(
        config: ShardConfig,
        mut receiver: mpsc::Receiver<RuntimeMessage>,
        output_tx: mpsc::Sender<A::Output>,
        scheduler: Arc<dyn Scheduler>,
        clock: Arc<dyn Clock>,
        id_gen: Arc<AtomicU64>,
        session_registry: SessionRegistry<A::SessionHandle>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(async move {
            let mut ready_queue: BTreeMap<DeviceKey, VecDeque<RuntimeMessage>> = BTreeMap::new();
            let mut actors: BTreeMap<DeviceKey, A> = BTreeMap::new();
            let ctx = ShardContext::new(
                config,
                output_tx,
                scheduler,
                clock,
                id_gen,
                session_registry,
            );

            loop {
                while !ready_queue.is_empty() {
                    process_ready_queue(&mut ready_queue, &mut actors, &ctx).await;
                }

                match receiver.recv().await {
                    Some(RuntimeMessage::Shutdown) => {
                        while !ready_queue.is_empty() {
                            process_ready_queue(&mut ready_queue, &mut actors, &ctx).await;
                        }
                        shutdown_all(actors, &ctx).await;
                        break;
                    }
                    Some(msg) => {
                        if let Some(device_key) = msg.device_key() {
                            ready_queue.entry(device_key).or_default().push_back(msg);
                        }
                    }
                    None => {
                        shutdown_all(actors, &ctx).await;
                        break;
                    }
                }
            }
        })
    }
}

async fn process_ready_queue<A: DeviceActor>(
    ready_queue: &mut BTreeMap<DeviceKey, VecDeque<RuntimeMessage>>,
    actors: &mut BTreeMap<DeviceKey, A>,
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
            }

            let queue_is_empty = queue.is_empty();
            if queue_is_empty {
                ready_queue.remove(&key);
            }
        }
    }
}

async fn shutdown_all<A: DeviceActor>(actors: BTreeMap<DeviceKey, A>, ctx: &ShardContext<A>) {
    for (key, actor) in actors {
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

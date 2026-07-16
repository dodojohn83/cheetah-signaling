//! Runtime entry point: starts shard workers and a timer wheel.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cheetah_runtime_api::{
    AdmissionController as AdmissionControllerTrait, DeviceActor, RuntimeConfig, RuntimeError,
    RuntimeMessage, Scheduler, SessionRegistry, ShardRouter,
};
use cheetah_signal_types::Clock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::admission::AdmissionController;
use crate::shard::{Shard, ShardConfig};
use crate::system_clock::SystemClock;
use crate::timer_scheduler::TimerScheduler;
use crate::timer_wheel::TimerWheel;

/// A runtime manages fixed shard workers and a timer wheel.
pub struct Runtime<A: DeviceActor> {
    inner: Arc<RuntimeInner<A>>,
}

struct RuntimeInner<A: DeviceActor> {
    config: RuntimeConfig,
    admission: AdmissionController,
    session_registry: SessionRegistry<A::SessionHandle>,
    timer_shutdown_tx: mpsc::Sender<()>,
    join_handles: Mutex<Option<Vec<JoinHandle<()>>>>,
}

impl<A: DeviceActor> std::fmt::Debug for Runtime<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

impl<A: DeviceActor> Clone for Runtime<A> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<A: DeviceActor> Runtime<A> {
    /// Starts the runtime and returns the runtime plus an output receiver.
    pub fn start(
        config: RuntimeConfig,
    ) -> Result<(Runtime<A>, mpsc::Receiver<A::Output>), RuntimeError> {
        config.validate()?;

        let (output_tx, output_rx) = mpsc::channel(config.output_channel_capacity);
        let (timer_tx, timer_rx) = mpsc::channel(config.timer_command_channel_capacity);
        let (timer_shutdown_tx, timer_shutdown_rx) = mpsc::channel(1);

        let router = ShardRouter::new(config.shard_count);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let scheduler: Arc<dyn Scheduler> = Arc::new(TimerScheduler::new(timer_tx));
        let id_gen = Arc::new(AtomicU64::new(1));
        let session_registry = SessionRegistry::new(config.max_sessions);

        let mut shard_senders = Vec::with_capacity(config.shard_count);
        let mut join_handles = Vec::with_capacity(config.shard_count + 1);

        let actor_config = config.actor_config.clone();
        for _ in 0..config.shard_count {
            let (sender, receiver) = mpsc::channel(config.shard_mailbox_capacity);
            shard_senders.push(sender);
            join_handles.push(tokio::spawn(Shard::run::<A>(
                ShardConfig::from(&config),
                receiver,
                output_tx.clone(),
                scheduler.clone(),
                clock.clone(),
                id_gen.clone(),
                session_registry.clone(),
                actor_config.clone(),
            )));
        }

        let senders = Arc::new(shard_senders);
        let admission = AdmissionController::new(
            router.clone(),
            senders.clone(),
            config.shard_mailbox_capacity,
        );

        join_handles.push(tokio::spawn(TimerWheel::run(
            timer_rx,
            timer_shutdown_rx,
            senders,
            router,
            config.timer_tick_resolution_ms,
            config.max_pending_dispatch,
        )));

        let inner = Arc::new(RuntimeInner {
            config,
            admission,
            session_registry,
            timer_shutdown_tx,
            join_handles: Mutex::new(Some(join_handles)),
        });

        Ok((Runtime { inner }, output_rx))
    }

    /// Sends a message to the runtime for processing by the appropriate shard.
    pub fn send_message(
        &self,
        device_key: cheetah_runtime_api::DeviceKey,
        message: RuntimeMessage,
    ) -> Result<(), RuntimeError> {
        if matches!(message, RuntimeMessage::Shutdown) {
            return Err(RuntimeError::InvalidArgument(
                "use shutdown() to stop the runtime".into(),
            ));
        }
        self.inner.admission.send(device_key, message)
    }

    /// Shuts down the runtime and waits for workers to finish.
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let handles = self
            .inner
            .join_handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();

        for sender in self.inner.admission.senders().iter() {
            let sender = sender.clone();
            match tokio::time::timeout(
                Duration::from_millis(100),
                sender.send(RuntimeMessage::Shutdown),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {}
                Err(_) => {
                    return Err(RuntimeError::Internal(
                        "timed out sending shutdown to shard".into(),
                    ));
                }
            }
        }

        let timer_shutdown = self.inner.timer_shutdown_tx.clone();
        match tokio::time::timeout(Duration::from_millis(100), timer_shutdown.send(())).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {}
            Err(_) => {
                return Err(RuntimeError::Internal(
                    "timed out sending shutdown to timer wheel".into(),
                ));
            }
        }

        if let Some(handles) = handles {
            for handle in handles {
                match tokio::time::timeout(Duration::from_secs(5), handle).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        return Err(RuntimeError::Internal(e.to_string()));
                    }
                    Err(_) => {
                        return Err(RuntimeError::Internal(
                            "timed out waiting for worker to stop".into(),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// Returns the runtime configuration.
    pub fn config(&self) -> &RuntimeConfig {
        &self.inner.config
    }

    /// Returns the session registry.
    pub fn session_registry(&self) -> &SessionRegistry<A::SessionHandle> {
        &self.inner.session_registry
    }

    /// Returns the admission controller.
    pub fn admission(&self) -> &dyn AdmissionControllerTrait {
        &self.inner.admission
    }
}

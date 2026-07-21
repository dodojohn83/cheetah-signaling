//! Integration tests for the Tokio runtime.

use std::collections::HashSet;

use cheetah_domain::{
    Command, CommandPayload, MediaPurpose, Operation,
    in_memory::{InMemoryClock, InMemoryIdGenerator},
};
use cheetah_runtime_api::{
    DeviceActor, DeviceKey, RuntimeConfig, RuntimeError, RuntimeMessage, SessionKey, TimerId,
};
use cheetah_runtime_tokio::Runtime;
use cheetah_signal_types::{
    ChannelId, CorrelationId, DeviceId, DurationMs, MediaSessionId, MessageId, NodeId, OwnerEpoch,
    Principal, PrincipalKind, ProtocolSessionId, RequestContext, ResourceId, ResourceKind,
    ResourceRef, TenantId,
};

use async_trait::async_trait;

#[derive(Default)]
struct FakeActor {
    timers: HashSet<TimerId>,
}

#[async_trait]
impl DeviceActor for FakeActor {
    type SessionHandle = String;
    type Output = String;
    type Error = RuntimeError;

    fn create(
        _ctx: cheetah_runtime_api::ActorContext<Self::SessionHandle>,
    ) -> Result<Self, Self::Error> {
        Ok(Self::default())
    }

    async fn handle(
        &mut self,
        message: RuntimeMessage,
        ctx: &cheetah_runtime_api::ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error> {
        match message {
            RuntimeMessage::ProtocolEvent { payload, .. } => {
                let timer_id = ctx
                    .schedule_timer(DurationMs::from_millis(50), "heartbeat")
                    .await?;
                self.timers.insert(timer_id);
                let session_key = SessionKey::new(ctx.tenant_id(), ProtocolSessionId::generate());
                ctx.session_registry()
                    .insert(session_key, "handle".to_string())?;
                Ok(vec![format!("event:{}", payload.len())])
            }
            RuntimeMessage::Command { command, .. } => {
                Ok(vec![format!("command:{}", command.payload().kind())])
            }
            RuntimeMessage::Timer { timer_id, kind, .. } => {
                if self.timers.remove(&timer_id) {
                    Ok(vec![format!("timer:{kind}")])
                } else {
                    Ok(vec![])
                }
            }
            RuntimeMessage::OwnershipChanged { owner_epoch, .. } => {
                Ok(vec![format!("owner:{}", owner_epoch.0)])
            }
            RuntimeMessage::Shutdown => Ok(vec!["shutdown-handle".into()]),
        }
    }

    async fn shutdown(
        self,
        _ctx: &cheetah_runtime_api::ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error> {
        Ok(vec!["shutdown".into()])
    }
}

#[tokio::test]
async fn runtime_sends_protocol_event() -> Result<(), RuntimeError> {
    let config = RuntimeConfig::default();
    let (runtime, mut output_rx) = Runtime::<FakeActor>::start(config)?;
    let (tenant_id, device_id) = (TenantId::generate(), DeviceId::generate());
    let key = DeviceKey::new(tenant_id, device_id);
    runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![1, 2, 3],
        },
    )?;
    let output = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no output".into()))?;
    assert_eq!(output, "event:3");
    runtime.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn runtime_sends_command() -> Result<(), RuntimeError> {
    let config = RuntimeConfig::default();
    let (runtime, mut output_rx) = Runtime::<FakeActor>::start(config)?;
    let (tenant_id, device_id) = (TenantId::generate(), DeviceId::generate());
    let key = DeviceKey::new(tenant_id, device_id);
    let command = make_command(device_id, tenant_id);
    runtime.send_message(
        key,
        RuntimeMessage::Command {
            device_key: key,
            command: Box::new(command),
        },
    )?;
    let output = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no output".into()))?;
    assert_eq!(output, "command:StartLive");
    runtime.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn runtime_schedules_timer() -> Result<(), RuntimeError> {
    let config = RuntimeConfig::default();
    let (runtime, mut output_rx) = Runtime::<FakeActor>::start(config)?;
    let (tenant_id, device_id) = (TenantId::generate(), DeviceId::generate());
    let key = DeviceKey::new(tenant_id, device_id);
    runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![1, 2, 3],
        },
    )?;
    let event = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no output".into()))?;
    assert_eq!(event, "event:3");

    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let timer = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no timer".into()))?;
    assert_eq!(timer, "timer:heartbeat");
    runtime.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn session_registry_is_shared_with_actor() -> Result<(), RuntimeError> {
    let config = RuntimeConfig::default();
    let (runtime, mut output_rx) = Runtime::<FakeActor>::start(config)?;
    let (tenant_id, device_id) = (TenantId::generate(), DeviceId::generate());
    let key = DeviceKey::new(tenant_id, device_id);
    runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![1],
        },
    )?;
    let output = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no output".into()))?;
    assert_eq!(output, "event:1");

    let list = runtime.session_registry().list(tenant_id);
    assert_eq!(list, vec!["handle".to_string()]);

    runtime.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn admission_controller_rejects_overload() -> Result<(), RuntimeError> {
    let config = RuntimeConfig {
        shard_count: 1,
        shard_mailbox_capacity: 1,
        output_channel_capacity: 1,
        ..Default::default()
    };
    let (runtime, mut output_rx) = Runtime::<FakeActor>::start(config)?;
    let (tenant_id, device_id) = (TenantId::generate(), DeviceId::generate());
    let key = DeviceKey::new(tenant_id, device_id);

    runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![1],
        },
    )?;
    tokio::task::yield_now().await;

    runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![2, 3],
        },
    )?;
    let result = runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![3, 4, 5],
        },
    );
    assert!(matches!(result, Err(RuntimeError::Overloaded)));

    let output1 = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no output 1".into()))?;
    let output2 = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no output 2".into()))?;
    assert_eq!(output1, "event:1");
    assert_eq!(output2, "event:2");

    drop(output_rx);
    runtime.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn shutdown_drains_actor() -> Result<(), RuntimeError> {
    let config = RuntimeConfig::default();
    let (runtime, mut output_rx) = Runtime::<FakeActor>::start(config)?;
    let (tenant_id, device_id) = (TenantId::generate(), DeviceId::generate());
    let key = DeviceKey::new(tenant_id, device_id);
    runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![1],
        },
    )?;
    let _ = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no output".into()))?;
    runtime.shutdown().await?;
    assert_eq!(output_rx.recv().await, Some("shutdown".to_string()));
    assert!(output_rx.recv().await.is_none());
    Ok(())
}

#[tokio::test]
async fn runtime_exposes_health_metrics() -> Result<(), RuntimeError> {
    let config = RuntimeConfig::default();
    let (runtime, mut output_rx) = Runtime::<FakeActor>::start(config)?;
    let (tenant_id, device_id) = (TenantId::generate(), DeviceId::generate());
    let key = DeviceKey::new(tenant_id, device_id);
    runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: vec![1, 2, 3],
        },
    )?;
    let _ = output_rx
        .recv()
        .await
        .ok_or_else(|| RuntimeError::Internal("no output".into()))?;

    let metrics = runtime.metrics();
    assert!(metrics.messages_enqueued >= 1);
    assert!(metrics.messages_processed >= 1);
    assert_eq!(metrics.actors_created, 1);
    assert_eq!(metrics.active_actors, 1);

    runtime.shutdown().await?;
    Ok(())
}

/// Actor that schedules a configurable number of timers from a single event and
/// emits one output per fired timer.
#[derive(Default)]
struct TimerStormActor;

#[async_trait]
impl DeviceActor for TimerStormActor {
    type SessionHandle = ();
    type Output = ();
    type Error = RuntimeError;

    fn create(
        _ctx: cheetah_runtime_api::ActorContext<Self::SessionHandle>,
    ) -> Result<Self, Self::Error> {
        Ok(Self)
    }

    async fn handle(
        &mut self,
        message: RuntimeMessage,
        ctx: &cheetah_runtime_api::ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error> {
        match message {
            RuntimeMessage::ProtocolEvent { payload, .. } => {
                let count = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                for _ in 0..count {
                    ctx.schedule_timer(DurationMs::from_millis(10), "storm")
                        .await?;
                }
                Ok(vec![])
            }
            RuntimeMessage::Timer { .. } => Ok(vec![()]),
            _ => Ok(vec![]),
        }
    }

    async fn shutdown(
        self,
        _ctx: &cheetah_runtime_api::ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error> {
        Ok(vec![])
    }
}

#[tokio::test(start_paused = true)]
async fn timer_wheel_fires_many_timers_under_paused_time() -> Result<(), RuntimeError> {
    const N: usize = 100_000;
    let config = RuntimeConfig {
        shard_count: 1,
        shard_mailbox_capacity: N + 16,
        timer_command_channel_capacity: N + 16,
        output_channel_capacity: 1024,
        timer_tick_resolution_ms: 1,
        max_pending_dispatch: N + 16,
        actor_idle_timeout_ms: 0,
        ..Default::default()
    };
    let (runtime, mut output_rx) = Runtime::<TimerStormActor>::start(config)?;
    let (tenant_id, device_id) = (TenantId::generate(), DeviceId::generate());
    let key = DeviceKey::new(tenant_id, device_id);

    runtime.send_message(
        key,
        RuntimeMessage::ProtocolEvent {
            device_key: key,
            payload: (N as u32).to_le_bytes().to_vec(),
        },
    )?;

    let mut fired = 0usize;
    while fired < N {
        output_rx
            .recv()
            .await
            .ok_or_else(|| RuntimeError::Internal("timer output channel closed".into()))?;
        fired += 1;
    }
    assert_eq!(fired, N);

    let metrics = runtime.metrics();
    assert_eq!(metrics.timers_scheduled, N as u64);
    assert_eq!(metrics.timers_fired, N as u64);

    runtime.shutdown().await?;
    Ok(())
}

fn make_command(device_id: DeviceId, tenant_id: TenantId) -> Command {
    let id_generator = InMemoryIdGenerator::new();
    let clock = InMemoryClock::new();
    let principal = Principal {
        id: "test".into(),
        kind: PrincipalKind::User,
        scopes: Vec::new(),
    };
    let context = RequestContext {
        tenant_id,
        principal,
        message_id: MessageId::generate(),
        correlation_id: CorrelationId::generate(),
        traceparent: None,
        tracestate: None,
        deadline: None,
        node_id: None,
        source_ip: None,
    };
    let target = ResourceRef {
        tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    };
    let payload = CommandPayload::StartLive {
        media_session_id: MediaSessionId::generate(),
        channel_id: ChannelId::generate(),
        media_node_id: NodeId::generate(),
        purpose: MediaPurpose::Unknown,
    };
    let (operation, _event) = Operation::new(
        &id_generator,
        &clock,
        &context,
        "key",
        device_id,
        target,
        payload,
        None,
        OwnerEpoch::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    operation.command().clone()
}

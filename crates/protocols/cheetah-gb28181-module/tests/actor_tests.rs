//! Integration tests for the GB28181 `DeviceActor` wrapper.

use async_trait::async_trait;
use cheetah_gb28181_core::{SipMessage, encode_message};
use cheetah_gb28181_module::{
    Gb28181Config,
    actor::Gb28181Actor,
    output::{Gb28181Heartbeat, Gb28181Output, Gb28181Register},
};
use cheetah_runtime_api::{ActorContext, DeviceActor, RuntimeError, RuntimeMessage, Scheduler};
use cheetah_signal_types::{Clock, DeviceId, DurationMs, TenantId, UtcTimestamp};
use std::any::Any;
use std::sync::{Arc, Mutex, atomic::AtomicU64};

mod common;
use common::{
    DEVICE_ID, authorization_for_challenge, message_request, now, register_request, source_addr,
};

#[derive(Clone)]
struct FakeClock;

impl Clock for FakeClock {
    fn now_wall(&self) -> UtcTimestamp {
        now()
    }

    fn now_monotonic(&self) -> DurationMs {
        DurationMs::default()
    }
}

#[derive(Clone, Debug)]
struct ScheduleEvent {
    timer_id: cheetah_runtime_api::TimerId,
    cancelled: bool,
}

#[derive(Clone)]
struct RecordingScheduler {
    events: Arc<Mutex<Vec<ScheduleEvent>>>,
}

impl RecordingScheduler {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn scheduled_count(&self) -> usize {
        self.events.lock().map_or(0, |g| g.len())
    }

    async fn cancel_count(&self) -> usize {
        self.events
            .lock()
            .map_or(0, |g| g.iter().filter(|e| e.cancelled).count())
    }

    async fn last_timer_id(&self) -> Option<cheetah_runtime_api::TimerId> {
        self.events
            .lock()
            .ok()
            .and_then(|g| g.iter().rfind(|e| !e.cancelled).map(|e| e.timer_id))
    }
}

#[async_trait]
impl Scheduler for RecordingScheduler {
    async fn schedule(
        &self,
        _device_key: cheetah_runtime_api::DeviceKey,
        timer_id: cheetah_runtime_api::TimerId,
        _delay: DurationMs,
        _kind: String,
    ) -> Result<(), RuntimeError> {
        let mut events = self
            .events
            .lock()
            .map_err(|_| RuntimeError::Internal("scheduler lock poisoned".into()))?;
        events.push(ScheduleEvent {
            timer_id,
            cancelled: false,
        });
        Ok(())
    }

    async fn cancel(
        &self,
        _device_key: cheetah_runtime_api::DeviceKey,
        timer_id: cheetah_runtime_api::TimerId,
    ) -> Result<(), RuntimeError> {
        let mut events = self
            .events
            .lock()
            .map_err(|_| RuntimeError::Internal("scheduler lock poisoned".into()))?;
        for event in events.iter_mut() {
            if event.timer_id == timer_id {
                event.cancelled = true;
                return Ok(());
            }
        }
        Ok(())
    }
}

fn actor_context(
    config: Arc<Gb28181Config>,
    scheduler: Arc<RecordingScheduler>,
) -> ActorContext<()> {
    let device_key =
        cheetah_runtime_api::DeviceKey::new(TenantId::generate(), DeviceId::generate());
    let actor_config: Option<Arc<dyn Any + Send + Sync>> = {
        let c: Arc<dyn Any + Send + Sync> = config;
        Some(c)
    };
    let scheduler_dyn: Arc<dyn Scheduler> = scheduler;
    ActorContext::new(
        device_key,
        scheduler_dyn,
        Arc::new(FakeClock),
        Arc::new(AtomicU64::new(1)),
        cheetah_runtime_api::SessionRegistry::<()>::new(16),
        actor_config,
    )
}

fn payload_for_request(request: &SipMessage) -> Vec<u8> {
    let mut payload = source_addr().to_string().into_bytes();
    payload.push(0);
    payload.extend_from_slice(&encode_message(request));
    payload
}

#[tokio::test]
async fn actor_full_device_flow() -> Result<(), Box<dyn std::error::Error>> {
    let config = common::test_config();
    let scheduler = Arc::new(RecordingScheduler::new());
    let ctx = actor_context(config.clone(), scheduler.clone());
    let key = ctx.device_key();

    let mut actor = Gb28181Actor::create(ctx.clone())?;

    // Unauthenticated REGISTER -> 401 challenge.
    let unauth = register_request(1, 3600, None)?;
    let outputs = actor
        .handle(
            RuntimeMessage::ProtocolEvent {
                device_key: key,
                payload: payload_for_request(&unauth),
            },
            &ctx,
        )
        .await?;
    let challenge = common::extract_www_authenticate(&outputs).ok_or("missing challenge")?;
    assert!(!challenge.is_empty());

    // Authenticated REGISTER -> Register output + heartbeat timer scheduled.
    let challenge = cheetah_gb28181_core::DigestChallenge::parse(&challenge)?;
    let auth = authorization_for_challenge(&challenge, 1, "abc123");
    let auth_request = register_request(2, 3600, Some(&auth))?;
    let outputs = actor
        .handle(
            RuntimeMessage::ProtocolEvent {
                device_key: key,
                payload: payload_for_request(&auth_request),
            },
            &ctx,
        )
        .await?;
    assert!(outputs.iter().any(|o| matches!(
        o,
        Gb28181Output::Register(Gb28181Register { external_id, .. }) if external_id == DEVICE_ID
    )));
    assert_eq!(scheduler.scheduled_count().await, 1);

    // Keepalive -> Heartbeat + timer reset.
    let body = b"<?xml version=\"1.0\"?><Notify><CmdType>Keepalive</CmdType><SN>1</SN><DeviceID>34020000001320000001</DeviceID><Status>OK</Status></Notify>";
    let request = message_request(1, body)?;
    let outputs = actor
        .handle(
            RuntimeMessage::ProtocolEvent {
                device_key: key,
                payload: payload_for_request(&request),
            },
            &ctx,
        )
        .await?;
    assert!(outputs.iter().any(|o| matches!(
        o,
        Gb28181Output::Heartbeat(Gb28181Heartbeat { status, .. }) if status == "OK"
    )));
    assert_eq!(scheduler.scheduled_count().await, 2);
    assert_eq!(scheduler.cancel_count().await, 1);

    // Heartbeat timeout clears registration.
    let timer_id = scheduler
        .last_timer_id()
        .await
        .ok_or("missing scheduled timer")?;
    let outputs = actor
        .handle(
            RuntimeMessage::Timer {
                device_key: key,
                timer_id,
                kind: "heartbeat_timeout".to_string(),
            },
            &ctx,
        )
        .await?;
    assert!(outputs.iter().any(|o| matches!(
        o,
        Gb28181Output::ProtocolError { kind, .. } if kind == "heartbeat_timeout"
    )));

    // Heartbeat timeout consumed the timer, so shutdown has nothing to cancel.
    actor.shutdown(&ctx).await?;
    assert_eq!(scheduler.cancel_count().await, 1);
    Ok(())
}

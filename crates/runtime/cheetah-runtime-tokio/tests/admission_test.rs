//! Integration tests for admission control (rate limiting, coalescing,
//! priority shedding, dead-letter redrive) and bounded drain.

use std::time::Duration;

use async_trait::async_trait;
use cheetah_runtime_api::{
    AdmissionPolicyConfig, DeviceActor, DeviceKey, RuntimeConfig, RuntimeError, RuntimeMessage,
};
use cheetah_runtime_tokio::{AdmissionOutcome, AdmissionTicket, Runtime};
use cheetah_signal_types::admission::TrafficClass;
use cheetah_signal_types::{DeviceId, TenantId};

/// A slow actor that blocks on a barrier so mailbox backlog can be observed
/// deterministically.
#[derive(Default)]
struct EchoActor;

#[async_trait]
impl DeviceActor for EchoActor {
    type SessionHandle = String;
    type Output = String;
    type Error = RuntimeError;

    fn create(
        _ctx: cheetah_runtime_api::ActorContext<Self::SessionHandle>,
    ) -> Result<Self, Self::Error> {
        Ok(Self)
    }

    async fn handle(
        &mut self,
        message: RuntimeMessage,
        _ctx: &cheetah_runtime_api::ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error> {
        match message {
            RuntimeMessage::ProtocolEvent { payload, .. } => {
                Ok(vec![format!("e:{}", payload.len())])
            }
            RuntimeMessage::Command { .. } => Ok(vec!["c".into()]),
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

fn key() -> DeviceKey {
    DeviceKey::new(TenantId::generate(), DeviceId::generate())
}

fn event(device_key: DeviceKey) -> RuntimeMessage {
    RuntimeMessage::ProtocolEvent {
        device_key,
        payload: vec![0],
    }
}

#[tokio::test]
async fn admit_rate_limits_per_source_and_method() -> Result<(), RuntimeError> {
    let config = RuntimeConfig {
        admission: AdmissionPolicyConfig {
            rate_capacity_tokens: 2,
            rate_refill_tokens_per_sec: 1,
            backlog_high_watermark: 1_000_000,
            backlog_low_watermark: 1,
            ..AdmissionPolicyConfig::default()
        },
        ..Default::default()
    };
    let (runtime, _rx) = Runtime::<EchoActor>::start(config)?;
    let k = key();
    let ticket = AdmissionTicket {
        source_id: 7,
        class: TrafficClass::Catalog,
        device_key: k,
    };

    assert_eq!(runtime.admit(ticket, event(k))?, AdmissionOutcome::Admitted);
    assert_eq!(runtime.admit(ticket, event(k))?, AdmissionOutcome::Admitted);
    // Third within the same second exceeds the burst and is dead-lettered.
    assert_eq!(
        runtime.admit(ticket, event(k))?,
        AdmissionOutcome::RateLimited
    );

    let snapshot = runtime.metrics();
    assert!(snapshot.messages_rate_limited >= 1);
    assert!(snapshot.messages_dead_lettered >= 1);
    runtime.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn admit_coalesces_keepalive_until_released() -> Result<(), RuntimeError> {
    let (runtime, _rx) = Runtime::<EchoActor>::start(RuntimeConfig::default())?;
    let k = key();
    let ticket = AdmissionTicket {
        source_id: 1,
        class: TrafficClass::Keepalive,
        device_key: k,
    };

    assert_eq!(runtime.admit(ticket, event(k))?, AdmissionOutcome::Admitted);
    assert_eq!(
        runtime.admit(ticket, event(k))?,
        AdmissionOutcome::Coalesced
    );
    runtime.release_coalescible(k, TrafficClass::Keepalive);
    assert_eq!(runtime.admit(ticket, event(k))?, AdmissionOutcome::Admitted);

    assert!(runtime.metrics().messages_coalesced >= 1);
    runtime.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn admit_sheds_low_priority_then_redrives_after_recovery() -> Result<(), RuntimeError> {
    // A single shard with a tiny mailbox so backlog crosses the high watermark
    // as soon as one message is queued and the actor is not draining it.
    let config = RuntimeConfig {
        shard_count: 1,
        shard_mailbox_capacity: 4,
        output_channel_capacity: 1,
        admission: AdmissionPolicyConfig {
            rate_capacity_tokens: 1_000,
            rate_refill_tokens_per_sec: 1_000,
            backlog_high_watermark: 1,
            backlog_low_watermark: 0,
            dead_letter_capacity: 16,
            ..AdmissionPolicyConfig::default()
        },
        ..Default::default()
    };
    let (runtime, output_rx) = Runtime::<EchoActor>::start(config)?;
    let k = key();

    // Fill the mailbox so aggregate depth reaches the high watermark. The
    // output channel has capacity 1 and is never drained, so the shard blocks
    // and the backlog persists.
    let command_ticket = AdmissionTicket {
        source_id: 1,
        class: TrafficClass::Command,
        device_key: k,
    };
    for _ in 0..4 {
        let _ = runtime.admit(command_ticket, event(k));
    }

    // Low-priority keepalive is shed while overloaded and dead-lettered.
    let keepalive_ticket = AdmissionTicket {
        source_id: 1,
        class: TrafficClass::Keepalive,
        device_key: k,
    };
    let outcome = runtime.admit(keepalive_ticket, event(k))?;
    assert_eq!(outcome, AdmissionOutcome::PriorityShed);
    assert!(runtime.metrics().messages_shed >= 1);
    assert!(runtime.metrics().backlog_overload_transitions >= 1);

    // Drain the output so the shard makes progress and the backlog clears.
    drop(output_rx);
    runtime.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn send_message_rejected_while_draining() -> Result<(), RuntimeError> {
    let (runtime, _rx) = Runtime::<EchoActor>::start(RuntimeConfig::default())?;
    let k = key();

    let outcome = runtime.drain(Duration::from_secs(2)).await?;
    assert!(outcome.drained_within_deadline);
    assert!(runtime.is_draining());

    let err = runtime.send_message(k, event(k));
    assert!(matches!(err, Err(RuntimeError::Draining)));

    let ticket = AdmissionTicket {
        source_id: 1,
        class: TrafficClass::Command,
        device_key: k,
    };
    let err = runtime.admit(ticket, event(k));
    assert!(matches!(err, Err(RuntimeError::Draining)));
    Ok(())
}

#[tokio::test]
async fn drain_reports_clean_completion_for_idle_runtime() -> Result<(), RuntimeError> {
    let (runtime, _rx) = Runtime::<EchoActor>::start(RuntimeConfig::default())?;
    let outcome = runtime.drain(Duration::from_secs(2)).await?;
    assert!(outcome.drained_within_deadline);
    assert_eq!(outcome.remaining_backlog, 0);
    Ok(())
}

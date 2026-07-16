//! GB28181 `DeviceActor` integration with the shard runtime.

use crate::{
    Gb28181Module, Gb28181ModuleError, config::Gb28181Config, module::Gb28181Input,
    output::Gb28181Output,
};
use async_trait::async_trait;
use cheetah_gb28181_core::{SipParser, SipParserConfig};
use cheetah_runtime_api::{ActorContext, DeviceActor, RuntimeError, RuntimeMessage};
use cheetah_signal_types::DurationMs;
use std::net::SocketAddr;
use std::sync::Arc;

/// Actor that runs one `Gb28181Module` per device key inside the runtime.
#[derive(Debug)]
pub struct Gb28181Actor {
    module: Gb28181Module,
    heartbeat_timer_id: Option<cheetah_runtime_api::TimerId>,
}

impl Gb28181Actor {
    /// Reschedules the heartbeat timeout timer after activity.
    async fn reset_heartbeat_timer(
        &mut self,
        ctx: &ActorContext<()>,
    ) -> Result<(), Gb28181ModuleError> {
        if let Some(id) = self.heartbeat_timer_id.take() {
            ctx.cancel_timer(id).await?;
        }
        let timeout_ms = i64::try_from(self.module.config().heartbeat_timeout_seconds)
            .unwrap_or(i64::MAX)
            .saturating_mul(1000);
        let id = ctx
            .schedule_timer(DurationMs::from_millis(timeout_ms), "heartbeat_timeout")
            .await?;
        self.heartbeat_timer_id = Some(id);
        Ok(())
    }

    /// Cancels any pending heartbeat timer.
    async fn cancel_heartbeat_timer(&mut self, ctx: &ActorContext<()>) {
        if let Some(id) = self.heartbeat_timer_id.take() {
            let _ = ctx.cancel_timer(id).await;
        }
    }
}

#[async_trait]
impl DeviceActor for Gb28181Actor {
    type SessionHandle = ();
    type Output = Gb28181Output;
    type Error = Gb28181ModuleError;

    fn create(ctx: ActorContext<Self::SessionHandle>) -> Result<Self, Self::Error> {
        let config: Arc<Gb28181Config> = ctx
            .actor_config::<Gb28181Config>()
            .ok_or_else(|| RuntimeError::InvalidArgument("missing GB28181 config".into()))?;
        let module = Gb28181Module::new(ctx.device_key(), config)?;
        Ok(Self {
            module,
            heartbeat_timer_id: None,
        })
    }

    async fn handle(
        &mut self,
        message: RuntimeMessage,
        ctx: &ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error> {
        let now = ctx.clock().now_wall();
        match message {
            RuntimeMessage::ProtocolEvent { payload, .. } => {
                let input = decode_input(&payload)?;
                let outputs = self.module.handle(input, now)?;
                if should_reset_heartbeat(&outputs) {
                    self.reset_heartbeat_timer(ctx).await?;
                } else if should_cancel_heartbeat(&outputs) {
                    self.cancel_heartbeat_timer(ctx).await;
                }
                Ok(outputs)
            }
            RuntimeMessage::Command { command, .. } => self.module.handle_command(&command, now),
            RuntimeMessage::Timer { kind, .. } => {
                if kind == "heartbeat_timeout" {
                    self.heartbeat_timer_id = None;
                    Ok(self.module.heartbeat_timeout(now))
                } else {
                    Ok(vec![])
                }
            }
            RuntimeMessage::OwnershipChanged { .. } => {
                self.cancel_heartbeat_timer(ctx).await;
                self.module.reset();
                Ok(vec![])
            }
            RuntimeMessage::Shutdown => {
                self.cancel_heartbeat_timer(ctx).await;
                Ok(vec![])
            }
        }
    }

    async fn shutdown(
        mut self,
        ctx: &ActorContext<Self::SessionHandle>,
    ) -> Result<Vec<Self::Output>, Self::Error> {
        self.cancel_heartbeat_timer(ctx).await;
        Ok(vec![])
    }
}

fn decode_input(payload: &[u8]) -> Result<Gb28181Input, Gb28181ModuleError> {
    let nul = payload
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| Gb28181ModuleError::InvalidMessage("missing source delimiter".into()))?;
    let source_str = std::str::from_utf8(&payload[..nul])
        .map_err(|e| Gb28181ModuleError::InvalidMessage(format!("invalid source: {e}")))?;
    let source: SocketAddr = source_str
        .parse()
        .map_err(|e: std::net::AddrParseError| Gb28181ModuleError::InvalidMessage(e.to_string()))?;
    let message = SipParser::parse_datagram(&payload[nul + 1..], SipParserConfig::default())?;
    Ok(Gb28181Input { source, message })
}

fn should_reset_heartbeat(outputs: &[Gb28181Output]) -> bool {
    outputs.iter().any(|o| {
        matches!(
            o,
            Gb28181Output::Register(_) | Gb28181Output::Refresh(_) | Gb28181Output::Heartbeat(_)
        )
    })
}

fn should_cancel_heartbeat(outputs: &[Gb28181Output]) -> bool {
    outputs
        .iter()
        .any(|o| matches!(o, Gb28181Output::Deregister))
}

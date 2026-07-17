//! Protocol driver port adapter for the ONVIF module.
//!
//! The ONVIF business logic lives in `devin/phase-17-onvif-services` and is not
//! yet merged to `main`. This driver is a minimal port adapter that exposes the
//! shared [`cheetah_plugin_sdk::ProtocolDriver`] trait so the plugin host can
//! register the factory and manage its lifecycle. Command dispatch will be
//! wired once the module implementation is available.

use async_trait::async_trait;
use cheetah_plugin_sdk::{
    CapabilityDescriptor, DriverCommand, DriverContext, HealthReport, HealthStatus, PluginError,
    PluginName, ProtocolCapability, ProtocolDirection, ProtocolDriver, ProtocolDriverFactory,
};
use cheetah_signal_types::DurationMs;
use std::collections::HashMap;
use std::net::SocketAddr;

/// Built-in ONVIF protocol driver placeholder.
#[derive(Debug)]
pub struct OnvifProtocolDriver;

impl OnvifProtocolDriver {
    /// Creates a new driver placeholder.
    pub fn new() -> Self {
        Self
    }
}

impl Default for OnvifProtocolDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProtocolDriver for OnvifProtocolDriver {
    async fn start(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    async fn drain(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    async fn shutdown(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    async fn handle_command(
        &self,
        _ctx: &dyn DriverContext,
        command: DriverCommand,
        _timeout: DurationMs,
    ) -> Result<(), PluginError> {
        Err(PluginError::Unsupported(format!(
            "ONVIF command {} is not yet implemented",
            command.command_type
        )))
    }

    async fn probe(
        &self,
        _ctx: &dyn DriverContext,
        target: &str,
        _timeout: DurationMs,
    ) -> Result<CapabilityDescriptor, PluginError> {
        let _ = target
            .parse::<SocketAddr>()
            .map_err(|e| PluginError::Driver(format!("invalid target address: {e}")))?;
        Ok(CapabilityDescriptor {
            protocol: "onvif".to_string(),
            direction: ProtocolDirection::Outbound,
            metadata: HashMap::new(),
        })
    }

    async fn health(
        &self,
        _ctx: &dyn DriverContext,
        _timeout: DurationMs,
    ) -> Result<HealthReport, PluginError> {
        Ok(HealthReport {
            status: HealthStatus::Degraded,
            message: "ONVIF implementation is not yet available".to_string(),
            metrics: HashMap::new(),
        })
    }
}

/// Factory for creating [`OnvifProtocolDriver`] instances.
#[derive(Debug)]
pub struct OnvifDriverFactory;

impl OnvifDriverFactory {
    /// Creates a new factory.
    pub fn new() -> Self {
        Self
    }
}

impl Default for OnvifDriverFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProtocolDriverFactory for OnvifDriverFactory {
    #[allow(clippy::expect_used)]
    fn name(&self) -> PluginName {
        PluginName::new("cheetah/onvif").expect("valid built-in plugin name")
    }

    fn capabilities(&self) -> Vec<ProtocolCapability> {
        vec![ProtocolCapability {
            protocol: "onvif".to_string(),
            direction: ProtocolDirection::Outbound,
            media_transport: None,
        }]
    }

    async fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn ProtocolDriver>, PluginError> {
        Ok(Box::new(OnvifProtocolDriver::new()))
    }
}

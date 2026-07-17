//! Plugin host lifecycle and built-in driver management.

use crate::error::PluginHostError;
use crate::loader::{ManifestLoader, ValidatedManifest};
use crate::registry::BuiltInRegistry;
use async_trait::async_trait;
use cheetah_plugin_sdk::{
    CommandSource, DeviceSink, DriverCommand, DriverContext, HealthReport, HealthStatus,
    PluginError, PluginManifest, PluginName, ProtocolDriver, ProtocolDriverFactory, ProtocolEvent,
    ResourceBudget,
};
use cheetah_signal_types::{DurationMs, PluginId, UtcTimestamp};
use secrecy::SecretString;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::time::timeout;

/// Source for tenant-scoped secrets referenced by drivers.
#[async_trait]
pub trait SecretProvider: Send + Sync {
    /// Returns the named secret if it exists and access is allowed.
    async fn get_secret(&self, name: &str) -> Result<Option<SecretString>, PluginError>;
}

/// A secret provider that never returns a secret.
#[derive(Debug)]
pub struct NoopSecretProvider;

#[async_trait]
impl SecretProvider for NoopSecretProvider {
    async fn get_secret(&self, _name: &str) -> Result<Option<SecretString>, PluginError> {
        Ok(None)
    }
}

/// A running plugin instance.
struct DriverInstance {
    #[allow(dead_code)]
    manifest: ValidatedManifest,
    driver: Box<dyn ProtocolDriver>,
    context: HostDriverContext,
}

/// Host-provided context passed to every driver call.
#[derive(Clone)]
pub struct HostDriverContext {
    plugin_name: PluginName,
    config: serde_json::Value,
    budget: ResourceBudget,
    monotonic_start: std::time::Instant,
    sink: Arc<dyn DeviceSink>,
    source: Arc<dyn CommandSource>,
    secret_provider: Arc<dyn SecretProvider>,
}

impl HostDriverContext {
    /// Creates a new host context.
    pub fn new(
        plugin_name: PluginName,
        config: serde_json::Value,
        budget: ResourceBudget,
        sink: Arc<dyn DeviceSink>,
        source: Arc<dyn CommandSource>,
    ) -> Self {
        Self::with_secret_provider(
            plugin_name,
            config,
            budget,
            sink,
            source,
            Arc::new(NoopSecretProvider),
        )
    }

    /// Creates a host context with a secret provider for driver lookups.
    pub fn with_secret_provider(
        plugin_name: PluginName,
        config: serde_json::Value,
        budget: ResourceBudget,
        sink: Arc<dyn DeviceSink>,
        source: Arc<dyn CommandSource>,
        secret_provider: Arc<dyn SecretProvider>,
    ) -> Self {
        Self {
            plugin_name,
            config,
            budget,
            monotonic_start: std::time::Instant::now(),
            sink,
            source,
            secret_provider,
        }
    }
}

#[async_trait]
impl DriverContext for HostDriverContext {
    fn plugin_name(&self) -> &PluginName {
        &self.plugin_name
    }

    fn config(&self) -> &serde_json::Value {
        &self.config
    }

    fn budget(&self) -> &ResourceBudget {
        &self.budget
    }

    fn monotonic_now(&self) -> cheetah_plugin_sdk::MonotonicSeconds {
        self.monotonic_start.elapsed().as_secs()
    }

    fn device_sink(&self) -> &dyn DeviceSink {
        self.sink.as_ref()
    }

    fn command_source(&self) -> &dyn CommandSource {
        self.source.as_ref()
    }

    async fn secret(&self, name: &str) -> Result<Option<SecretString>, PluginError> {
        self.secret_provider.get_secret(name).await
    }

    async fn request_media_session(
        &self,
        _params: serde_json::Value,
        _timeout: DurationMs,
    ) -> Result<String, PluginError> {
        Err(PluginError::Unsupported(
            "media session request".to_string(),
        ))
    }

    async fn register_endpoint(
        &self,
        _protocol: &str,
        _address: &str,
    ) -> Result<String, PluginError> {
        Err(PluginError::Unsupported(
            "endpoint registration".to_string(),
        ))
    }
}

impl fmt::Debug for HostDriverContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostDriverContext")
            .field("plugin_name", &self.plugin_name)
            .field("config", &self.config)
            .field("budget", &self.budget)
            .field("sink", &"<dyn DeviceSink>")
            .field("source", &"<dyn CommandSource>")
            .field("secret_provider", &"<dyn SecretProvider>")
            .finish()
    }
}

/// Manages built-in plugin drivers.
pub struct PluginHost {
    loader: ManifestLoader,
    registry: BuiltInRegistry,
    instances: HashMap<PluginId, DriverInstance>,
    default_timeout: DurationMs,
    secret_provider: Arc<dyn SecretProvider>,
}

/// Minimum driver operation timeout. Non-positive explicit timeouts are
/// coerced to this value so every lifecycle call remains bounded.
const MIN_DRIVER_TIMEOUT: DurationMs = DurationMs::from_millis(1_000);

impl PluginHost {
    /// Creates a new plugin host for the given SDK version and a default
    /// no-op secret provider.
    pub fn new(host_sdk_version: semver::Version, default_timeout: DurationMs) -> Self {
        Self::with_secret_provider(
            host_sdk_version,
            default_timeout,
            Arc::new(NoopSecretProvider),
        )
    }

    /// Creates a new plugin host with the given secret provider.
    pub fn with_secret_provider(
        host_sdk_version: semver::Version,
        default_timeout: DurationMs,
        secret_provider: Arc<dyn SecretProvider>,
    ) -> Self {
        let default_timeout = if default_timeout.as_millis() <= 0 {
            MIN_DRIVER_TIMEOUT
        } else {
            default_timeout
        };
        Self {
            loader: ManifestLoader::new(host_sdk_version),
            registry: BuiltInRegistry::new(),
            instances: HashMap::new(),
            default_timeout,
            secret_provider,
        }
    }

    /// Registers a built-in factory.
    pub fn register_builtin(
        &mut self,
        name: PluginName,
        factory: Box<dyn ProtocolDriverFactory>,
    ) -> Result<(), PluginHostError> {
        self.registry.register(name, factory)
    }

    /// Validates a manifest without activating it.
    pub fn validate_manifest(
        &self,
        manifest: &PluginManifest,
        payload: &[u8],
    ) -> Result<ValidatedManifest, PluginHostError> {
        self.loader.validate(manifest, payload)
    }

    /// Activates a plugin instance.
    ///
    /// Validates the manifest, creates a driver using the registered factory,
    /// starts it, and stores the instance. On failure the previous instance is
    /// preserved.
    #[allow(clippy::too_many_arguments)]
    pub async fn activate(
        &mut self,
        id: PluginId,
        manifest: &PluginManifest,
        payload: &[u8],
        config: serde_json::Value,
        sink: Arc<dyn DeviceSink>,
        source: Arc<dyn CommandSource>,
        timeout: Option<DurationMs>,
    ) -> Result<(), PluginHostError> {
        if self.instances.contains_key(&id) {
            return Err(PluginHostError::InstanceExists(id.to_string()));
        }

        let validated = self.loader.validate(manifest, payload)?;
        let deadline = timeout.unwrap_or(self.default_timeout);
        let driver = {
            let factory = self
                .registry
                .get(&manifest.name)
                .ok_or_else(|| PluginHostError::NotFound(manifest.name.to_string()))?;
            let create_deadline = deadline.max(factory.creation_timeout());
            with_timeout(create_deadline, factory.create(config.clone())).await?
        };
        let ctx = HostDriverContext::with_secret_provider(
            manifest.name.clone(),
            config,
            manifest.resource_budget,
            sink,
            source,
            Arc::clone(&self.secret_provider),
        );

        with_timeout(deadline, driver.start(&ctx, deadline)).await?;

        self.instances.insert(
            id,
            DriverInstance {
                manifest: validated,
                driver,
                context: ctx,
            },
        );
        Ok(())
    }

    /// Drains an instance, stopping new work.
    pub async fn drain(
        &self,
        id: PluginId,
        timeout: Option<DurationMs>,
    ) -> Result<(), PluginHostError> {
        let deadline = timeout.unwrap_or(self.default_timeout);
        let instance = self
            .instances
            .get(&id)
            .ok_or_else(|| PluginHostError::NotFound(id.to_string()))?;
        with_timeout(deadline, instance.driver.drain(&instance.context, deadline)).await
    }

    /// Shuts down an instance and removes it.
    pub async fn shutdown(
        &mut self,
        id: PluginId,
        timeout: Option<DurationMs>,
    ) -> Result<(), PluginHostError> {
        let deadline = timeout.unwrap_or(self.default_timeout);
        let instance = self
            .instances
            .remove(&id)
            .ok_or_else(|| PluginHostError::NotFound(id.to_string()))?;
        with_timeout(
            deadline,
            instance.driver.shutdown(&instance.context, deadline),
        )
        .await
    }

    /// Dispatches a command to a driver instance.
    ///
    /// The command's `deadline` is used as a wall-clock timeout; if none was set
    /// (deadline is the Unix epoch) the host default is used.
    pub async fn handle_command(
        &self,
        id: PluginId,
        command: DriverCommand,
    ) -> Result<(), PluginHostError> {
        let instance = self
            .instances
            .get(&id)
            .ok_or_else(|| PluginHostError::NotFound(id.to_string()))?;
        let deadline = command_deadline(&command, self.default_timeout)?;
        with_timeout(
            deadline,
            instance
                .driver
                .handle_command(&instance.context, command, deadline),
        )
        .await
    }

    /// Probes a target by creating a temporary driver and calling its probe.
    pub async fn probe(
        &self,
        name: &PluginName,
        target: &str,
        timeout: Option<DurationMs>,
    ) -> Result<cheetah_plugin_sdk::CapabilityDescriptor, PluginHostError> {
        let deadline = timeout.unwrap_or(self.default_timeout);
        let factory = self
            .registry
            .get(name)
            .ok_or_else(|| PluginHostError::NotFound(name.to_string()))?;
        let create_deadline = deadline.max(factory.creation_timeout());
        let driver = with_timeout(create_deadline, factory.create(serde_json::Value::Null)).await?;
        let ctx = self.no_op_context(name);
        with_timeout(deadline, driver.probe(&ctx, target, deadline)).await
    }

    /// Aggregates health from all instances.
    pub async fn health(&self) -> HealthReport {
        if self.instances.is_empty() {
            return HealthReport {
                status: HealthStatus::Healthy,
                message: "no active plugin instances".to_string(),
                metrics: HashMap::new(),
            };
        }

        let mut reports = Vec::with_capacity(self.instances.len());
        for (id, instance) in &self.instances {
            match with_timeout(
                self.default_timeout,
                instance
                    .driver
                    .health(&instance.context, self.default_timeout),
            )
            .await
            {
                Ok(report) => reports.push(report),
                Err(_) => {
                    return HealthReport {
                        status: HealthStatus::Unhealthy,
                        message: format!("instance {id} health check failed"),
                        metrics: HashMap::new(),
                    };
                }
            }
        }

        let status = if reports.iter().all(|r| r.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else if reports.iter().any(|r| r.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Degraded
        };

        let mut metrics = HashMap::new();
        metrics.insert("instance_count".to_string(), reports.len() as u64);

        HealthReport {
            status,
            message: "aggregated built-in driver health".to_string(),
            metrics,
        }
    }

    /// Returns the number of active instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }
}

impl fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginHost")
            .field("instance_count", &self.instances.len())
            .field("default_timeout_ms", &self.default_timeout.as_millis())
            .finish()
    }
}

impl PluginHost {
    fn no_op_context(&self, name: &PluginName) -> HostDriverContext {
        HostDriverContext::with_secret_provider(
            name.clone(),
            serde_json::Value::Null,
            ResourceBudget::default(),
            Arc::new(NoOpSink),
            Arc::new(NoOpSource),
            Arc::clone(&self.secret_provider),
        )
    }
}

struct NoOpSink;

#[async_trait]
impl DeviceSink for NoOpSink {
    async fn emit_event(&self, _event: ProtocolEvent) -> Result<(), PluginError> {
        Ok(())
    }
}

struct NoOpSource;

#[async_trait]
impl CommandSource for NoOpSource {
    async fn next_command(&self) -> Result<Option<DriverCommand>, PluginError> {
        Ok(None)
    }
}

fn command_deadline(
    command: &DriverCommand,
    default: DurationMs,
) -> Result<DurationMs, PluginHostError> {
    let epoch = UtcTimestamp::default();
    if command.deadline == epoch {
        return Ok(default);
    }

    let now = time::OffsetDateTime::now_utc();
    let deadline = command.deadline.as_offset();
    let diff = deadline - now;
    let remaining_ms = diff.whole_milliseconds();
    if remaining_ms <= 0 {
        return Err(PluginHostError::Timeout);
    }
    let remaining_ms: i64 = remaining_ms
        .try_into()
        .map_err(|_| PluginHostError::Internal("command deadline overflow".to_string()))?;
    Ok(DurationMs::from_millis(remaining_ms))
}

async fn with_timeout<F, T>(deadline: DurationMs, fut: F) -> Result<T, PluginHostError>
where
    F: std::future::Future<Output = Result<T, PluginError>> + Send,
{
    let mut millis = deadline.as_millis();
    if millis <= 0 {
        millis = MIN_DRIVER_TIMEOUT.as_millis();
    }
    let std_duration = std::time::Duration::from_millis(millis as u64);
    timeout(std_duration, fut)
        .await
        .map_err(|_| PluginHostError::Timeout)?
        .map_err(Into::into)
}

//! Layered configuration loader for Cheetah Signaling.
//!
//! The default priority is:
//!
//! 1. Built in defaults (encoded as TOML).
//! 2. Optional TOML configuration file.
//! 3. Environment variables with the `CHEETAH_` prefix.
//! 4. Secret provider overrides applied to the resulting `SignalConfig`.

use std::path::{Path, PathBuf};

use cheetah_signal_types::{ConfigSource, Result, SignalConfig, SignalError, SignalErrorKind};
use config::{Config, File, FileFormat};

/// Layered configuration source.
#[derive(Clone, Debug, Default)]
pub struct LayeredConfigSource {
    /// Optional path to a TOML configuration file.
    config_path: Option<PathBuf>,
}

impl LayeredConfigSource {
    /// Creates a new source with no file override.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a TOML configuration file path.
    #[must_use]
    pub fn with_config_path(mut self, path: impl AsRef<Path>) -> Self {
        self.config_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Loads the configuration using the configured sources.
    ///
    /// If no explicit path is set, the `CHEETAH_CONFIG_PATH` environment
    /// variable is honored as a fallback.
    fn load(&self) -> Result<SignalConfig> {
        let config_path = self
            .config_path
            .clone()
            .or_else(|| std::env::var("CHEETAH_CONFIG_PATH").ok().map(PathBuf::from));

        let default_toml = SignalConfig::example_toml()?;

        let mut builder =
            Config::builder().add_source(File::from_str(&default_toml, FileFormat::Toml));

        if let Some(path) = config_path {
            let path_str = path.to_str().ok_or_else(|| {
                SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "config path is not valid UTF-8",
                )
            })?;
            builder = builder.add_source(File::new(path_str, FileFormat::Toml));
        }

        builder = builder.add_source(
            config::Environment::with_prefix("CHEETAH")
                .try_parsing(true)
                .separator("__"),
        );

        let cfg = builder.build().map_err(|e| {
            SignalError::new(SignalErrorKind::InvalidArgument, "failed to build config")
                .with_source(e)
        })?;

        let signal_config: SignalConfig = cfg.try_deserialize().map_err(|e| {
            SignalError::new(
                SignalErrorKind::InvalidArgument,
                "failed to deserialize config",
            )
            .with_source(e)
        })?;

        signal_config.validate()?;

        Ok(signal_config)
    }
}

impl ConfigSource for LayeredConfigSource {
    fn snapshot(&self) -> Result<SignalConfig> {
        self.load()
    }
}

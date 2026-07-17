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
use config::{Config, Environment, File, FileFormat};

/// Prefix used for configuration environment variables.
const CONFIG_ENV_PREFIX: &str = "CHEETAH_";

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
    fn load(&self) -> Result<SignalConfig> {
        let default_toml = SignalConfig::example_toml()?;

        let mut builder =
            Config::builder().add_source(File::from_str(&default_toml, FileFormat::Toml));

        if let Some(path) = &self.config_path {
            let path_str = path.to_str().ok_or_else(|| {
                SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "config path is not valid UTF-8",
                )
            })?;
            builder = builder.add_source(File::new(path_str, FileFormat::Toml));
        }

        builder = builder.add_source(env_source(std::env::vars()));

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

/// Builds an environment source from an iterator of `(key, value)` pairs.
///
/// Only keys starting with `CHEETAH_` are considered. Keys that do not contain
/// the section separator (`__`) are ignored, which prevents stray `CHEETAH_*`
/// variables (including `CHEETAH_SECRET_*` references used by the secret store)
/// from being deserialized as unknown top-level config fields.
fn env_source(vars: impl Iterator<Item = (String, String)>) -> Environment {
    let mut source = config::Map::new();
    for (key, value) in vars {
        if let Some(rest) = key.strip_prefix(CONFIG_ENV_PREFIX) {
            // Require at least one `__` separator so only nested config keys are
            // forwarded. This excludes CHEETAH_SECRET_* and other single-segment
            // environment variables.
            if rest.contains("__") {
                source.insert(key, value);
            }
        }
    }

    config::Environment::with_prefix("CHEETAH")
        .prefix_separator("_")
        .separator("__")
        .try_parsing(true)
        .source(Some(source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn example_config_round_trips() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let example = if let Some(root) = manifest.ancestors().nth(3) {
            root.join("config.example.toml")
        } else {
            panic!("workspace root ancestor not found");
        };
        let source = LayeredConfigSource::new().with_config_path(example);
        match source.snapshot() {
            Ok(config) => assert_eq!(config.http.port, 8080),
            Err(e) => panic!("example config should load: {e}"),
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let toml = r#"
[system]
node_name = "x"
unknown_field = true
"#;
        let mut builder = Config::builder();
        builder = builder.add_source(File::from_str(toml, FileFormat::Toml));
        let cfg = match builder.build() {
            Ok(c) => c,
            Err(e) => panic!("test config should build: {e}"),
        };
        let result: std::result::Result<SignalConfig, _> = cfg.try_deserialize();
        assert!(result.is_err());
    }

    #[test]
    fn env_override_changes_http_port() {
        let env = vec![("CHEETAH_HTTP__PORT".to_string(), "9090".to_string())];
        let source = env_source(env.into_iter());
        let cfg = match Config::builder().add_source(source).build() {
            Ok(c) => c,
            Err(e) => panic!("config build failed: {e}"),
        };
        let config: SignalConfig = match cfg.try_deserialize() {
            Ok(c) => c,
            Err(e) => panic!("config deserialize failed: {e}"),
        };
        assert_eq!(config.http.port, 9090);
    }

    #[test]
    fn env_secret_variables_are_ignored() {
        let env = vec![
            ("CHEETAH_SECRET_SIG_TEST".to_string(), "s3cr3t".to_string()),
            ("CHEETAH_HTTP__PORT".to_string(), "9090".to_string()),
        ];
        let source = env_source(env.into_iter());
        let cfg = match Config::builder().add_source(source).build() {
            Ok(c) => c,
            Err(e) => panic!("config build failed: {e}"),
        };
        let config: SignalConfig = match cfg.try_deserialize() {
            Ok(c) => c,
            Err(e) => panic!("config deserialize failed: {e}"),
        };
        assert_eq!(config.http.port, 9090);
    }

    #[test]
    fn env_top_level_unknown_keys_are_ignored() {
        let env = vec![
            ("CHEETAH_FOO".to_string(), "bar".to_string()),
            ("CHEETAH_HTTP__PORT".to_string(), "9090".to_string()),
        ];
        let source = env_source(env.into_iter());
        let cfg = match Config::builder().add_source(source).build() {
            Ok(c) => c,
            Err(e) => panic!("config build failed: {e}"),
        };
        let config: SignalConfig = match cfg.try_deserialize() {
            Ok(c) => c,
            Err(e) => panic!("config deserialize failed: {e}"),
        };
        assert_eq!(config.http.port, 9090);
    }
}

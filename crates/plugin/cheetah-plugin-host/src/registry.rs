//! Static built-in driver registry.

use crate::error::PluginHostError;
use cheetah_plugin_sdk::{PluginName, ProtocolDriverFactory};
use std::collections::HashMap;
use std::fmt;

/// Registry of built-in driver factories keyed by plugin name.
#[derive(Default)]
pub struct BuiltInRegistry {
    factories: HashMap<PluginName, Box<dyn ProtocolDriverFactory>>,
}

impl BuiltInRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a built-in factory.
    pub fn register(
        &mut self,
        name: PluginName,
        factory: Box<dyn ProtocolDriverFactory>,
    ) -> Result<(), PluginHostError> {
        if self.factories.contains_key(&name) {
            return Err(PluginHostError::AlreadyRegistered(name.to_string()));
        }
        self.factories.insert(name, factory);
        Ok(())
    }

    /// Returns the factory for a plugin name, if any.
    pub fn get(&self, name: &PluginName) -> Option<&dyn ProtocolDriverFactory> {
        self.factories.get(name).map(|f| f.as_ref())
    }

    /// Iterates over registered factories.
    pub fn iter(&self) -> impl Iterator<Item = (&PluginName, &dyn ProtocolDriverFactory)> {
        self.factories.iter().map(|(k, v)| (k, v.as_ref()))
    }

    /// Returns the number of registered factories.
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Returns whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

impl fmt::Debug for BuiltInRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuiltInRegistry")
            .field("count", &self.factories.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginHostError;
    use async_trait::async_trait;
    use cheetah_plugin_sdk::{PluginError, ProtocolCapability, ProtocolDriver};

    struct DummyFactory;

    #[async_trait]
    impl ProtocolDriverFactory for DummyFactory {
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        fn name(&self) -> PluginName {
            PluginName::new("test/dummy").expect("valid test plugin name")
        }

        fn capabilities(&self) -> Vec<ProtocolCapability> {
            vec![]
        }

        async fn create(
            &self,
            _config: serde_json::Value,
        ) -> Result<Box<dyn ProtocolDriver>, PluginError> {
            Err(PluginError::Unsupported("dummy".to_string()))
        }
    }

    #[test]
    fn register_and_get_factory() -> Result<(), PluginHostError> {
        let mut registry = BuiltInRegistry::new();
        let name = PluginName::new("test/dummy")?;
        registry.register(name.clone(), Box::new(DummyFactory))?;
        assert!(registry.get(&name).is_some());
        Ok(())
    }

    #[test]
    fn duplicate_registration_fails() -> Result<(), PluginHostError> {
        let mut registry = BuiltInRegistry::new();
        let name = PluginName::new("test/dummy")?;
        registry.register(name.clone(), Box::new(DummyFactory))?;
        assert!(registry.register(name, Box::new(DummyFactory)).is_err());
        Ok(())
    }
}

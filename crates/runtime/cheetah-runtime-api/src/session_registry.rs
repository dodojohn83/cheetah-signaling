//! Per-node session registry.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use cheetah_signal_types::TenantId;

use crate::{RuntimeError, SessionKey};

/// Holds protocol session and transaction handles for a runtime instance.
#[derive(Clone)]
pub struct SessionRegistry<Handle: Clone + Send + Sync + 'static> {
    inner: Arc<Mutex<BTreeMap<SessionKey, Handle>>>,
    max_sessions: usize,
}

impl<Handle: Clone + Send + Sync + 'static> std::fmt::Debug for SessionRegistry<Handle> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRegistry")
            .field("max_sessions", &self.max_sessions)
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl<Handle: Clone + Send + Sync + 'static> SessionRegistry<Handle> {
    /// Creates a new registry with the given session limit.
    pub fn new(max_sessions: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BTreeMap::new())),
            max_sessions,
        }
    }

    fn lock_guard(&self) -> std::sync::MutexGuard<'_, BTreeMap<SessionKey, Handle>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Returns the configured session limit.
    pub fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    /// Returns the handle for the given session key, if any.
    pub fn get(&self, key: &SessionKey) -> Option<Handle> {
        self.lock_guard().get(key).cloned()
    }

    /// Inserts a handle and returns the previous handle, if any.
    ///
    /// Returns `Overloaded` if the registry is at capacity and the key is new.
    pub fn insert(&self, key: SessionKey, handle: Handle) -> Result<Option<Handle>, RuntimeError> {
        let mut guard = self.lock_guard();
        if !guard.contains_key(&key) && guard.len() >= self.max_sessions {
            return Err(RuntimeError::Overloaded);
        }
        Ok(guard.insert(key, handle))
    }

    /// Removes and returns the handle for the given session key, if any.
    pub fn remove(&self, key: &SessionKey) -> Option<Handle> {
        self.lock_guard().remove(key)
    }

    /// Returns all handles for the given tenant.
    pub fn list(&self, tenant_id: TenantId) -> Vec<Handle> {
        self.lock_guard()
            .iter()
            .filter(|(key, _)| key.tenant_id() == tenant_id)
            .map(|(_, handle)| handle.clone())
            .collect()
    }

    /// Returns whether the registry contains the given key.
    pub fn contains(&self, key: &SessionKey) -> bool {
        self.lock_guard().contains_key(key)
    }

    /// Returns the number of registered handles.
    pub fn len(&self) -> usize {
        self.lock_guard().len()
    }

    /// Returns whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.lock_guard().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get_round_trip() {
        let registry = SessionRegistry::new(10);
        let key = SessionKey::new(
            cheetah_signal_types::TenantId::generate(),
            cheetah_signal_types::ProtocolSessionId::generate(),
        );
        assert!(matches!(
            registry.insert(key, "handle".to_string()),
            Ok(None)
        ));
        assert_eq!(registry.get(&key), Some("handle".to_string()));
    }

    #[test]
    fn insert_refuses_over_capacity() {
        let registry = SessionRegistry::new(1);
        let key1 = SessionKey::new(
            cheetah_signal_types::TenantId::generate(),
            cheetah_signal_types::ProtocolSessionId::generate(),
        );
        let key2 = SessionKey::new(
            cheetah_signal_types::TenantId::generate(),
            cheetah_signal_types::ProtocolSessionId::generate(),
        );
        assert!(matches!(registry.insert(key1, "a".to_string()), Ok(None)));
        assert!(registry.insert(key2, "b".to_string()).is_err());
    }

    #[test]
    fn list_filters_by_tenant() {
        let tenant = cheetah_signal_types::TenantId::generate();
        let other = cheetah_signal_types::TenantId::generate();
        let registry = SessionRegistry::new(10);
        let key1 = SessionKey::new(tenant, cheetah_signal_types::ProtocolSessionId::generate());
        let key2 = SessionKey::new(other, cheetah_signal_types::ProtocolSessionId::generate());
        assert!(matches!(registry.insert(key1, "a".to_string()), Ok(None)));
        assert!(matches!(registry.insert(key2, "b".to_string()), Ok(None)));
        let handles = registry.list(tenant);
        assert_eq!(handles, vec!["a".to_string()]);
    }
}

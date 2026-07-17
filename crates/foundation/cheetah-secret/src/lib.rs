//! Secret provider implementations for Cheetah Signaling.
//!
//! This crate provides concrete [`SecretStore`] implementations backed by
//! environment variables, files and an in-memory store. A `CompositeSecretStore`
//! lets callers layer sources so a running system can read from an external
//! secret manager while falling back to local files or memory in tests and
//! development.
//!
//! All providers use [`secrecy::SecretString`] so the secret value is hidden in
//! `Debug` output and dropped with zeroization when the `SecretString` is
//! released.

use cheetah_signal_types::{Result, SecretStore, SignalError, SignalErrorKind};
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// In-memory secret store for tests and development.
#[derive(Clone, Debug)]
pub struct InMemorySecretStore {
    secrets: Arc<Mutex<HashMap<String, SecretString>>>,
}

impl Default for InMemorySecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySecretStore {
    /// Creates an empty in-memory store.
    pub fn new() -> Self {
        Self {
            secrets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Inserts a secret directly; useful in tests.
    pub fn insert(&self, key: impl Into<String>, value: impl Into<String>) {
        let mut guard = lock(&self.secrets);
        guard.insert(key.into(), SecretString::from(value.into()));
    }
}

impl SecretStore for InMemorySecretStore {
    fn get(&self, key: &str) -> Result<SecretString> {
        let guard = lock(&self.secrets);
        guard
            .get(key)
            .cloned()
            .ok_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found"))
    }

    fn put(&self, key: &str, value: SecretString) -> Result<()> {
        let mut guard = lock(&self.secrets);
        guard.insert(key.to_string(), value);
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut guard = lock(&self.secrets);
        guard
            .remove(key)
            .map(|_| ())
            .ok_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found"))
    }

    fn rotate(&self, key: &str) -> Result<SecretString> {
        let mut guard = lock(&self.secrets);
        let previous = guard
            .get(key)
            .ok_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found"))?
            .expose_secret()
            .to_string();
        let rotated = SecretString::from(uuid::Uuid::new_v4().to_string());
        guard.insert(key.to_string(), rotated);
        Ok(SecretString::from(previous))
    }
}

/// Secret store backed by environment variables.
///
/// Key names are normalized to upper case and non-alphanumeric characters are
/// replaced with `_`. The configured prefix is prepended. For example, with the
/// default prefix `CHEETAH_SECRET_` the key `sig.test` is looked up as
/// `CHEETAH_SECRET_SIG_TEST`.
#[derive(Clone, Debug)]
pub struct EnvSecretStore {
    prefix: String,
}

impl Default for EnvSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvSecretStore {
    /// Creates an env store with the default `CHEETAH_SECRET_` prefix.
    pub fn new() -> Self {
        Self {
            prefix: "CHEETAH_SECRET_".to_string(),
        }
    }

    /// Creates an env store with a custom prefix.
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }

    fn env_key(&self, key: &str) -> String {
        let normalized: String = key
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        format!("{}{}", self.prefix, normalized)
    }
}

impl SecretStore for EnvSecretStore {
    fn get(&self, key: &str) -> Result<SecretString> {
        let env_key = self.env_key(key);
        match std::env::var(&env_key) {
            Ok(value) => Ok(SecretString::from(value)),
            Err(std::env::VarError::NotPresent) => Err(SignalError::new(
                SignalErrorKind::NotFound,
                format!("secret {key} not found in environment"),
            )),
            Err(std::env::VarError::NotUnicode(_)) => Err(SignalError::new(
                SignalErrorKind::Internal,
                format!("secret {key} is not valid UTF-8"),
            )),
        }
    }

    fn put(&self, _key: &str, _value: SecretString) -> Result<()> {
        Err(SignalError::new(
            SignalErrorKind::Unsupported,
            "env secret store is read-only",
        ))
    }

    fn delete(&self, _key: &str) -> Result<()> {
        Err(SignalError::new(
            SignalErrorKind::Unsupported,
            "env secret store is read-only",
        ))
    }

    fn rotate(&self, _key: &str) -> Result<SecretString> {
        Err(SignalError::new(
            SignalErrorKind::Unsupported,
            "env secret store is read-only",
        ))
    }
}

/// Secret store backed by a directory of files.
///
/// Each secret is a plain text file named after the key. Keys containing path
/// separators or `..` are rejected.
#[derive(Clone, Debug)]
pub struct FileSecretStore {
    base_dir: PathBuf,
}

impl FileSecretStore {
    /// Creates a file-backed store rooted at `base_dir`.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn secret_path(&self, key: &str) -> Result<PathBuf> {
        if key.contains('/') || key.contains('\\') || key.contains("..") {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "secret key contains path separators",
            ));
        }
        Ok(self.base_dir.join(key))
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, key: &str) -> Result<SecretString> {
        let path = self.secret_path(key)?;
        match std::fs::read_to_string(&path) {
            Ok(value) => Ok(SecretString::from(value.trim_end_matches('\n').to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SignalError::new(
                SignalErrorKind::NotFound,
                format!("secret {key} not found"),
            )),
            Err(e) => Err(SignalError::new(
                SignalErrorKind::Internal,
                format!("failed to read secret {key}"),
            )
            .with_source(e)),
        }
    }

    fn put(&self, key: &str, value: SecretString) -> Result<()> {
        let path = self.secret_path(key)?;
        create_secret_dir(&self.base_dir).map_err(|e| {
            SignalError::new(
                SignalErrorKind::Internal,
                "failed to create secret directory",
            )
            .with_source(e)
        })?;
        write_secret_file(&path, value.expose_secret().as_bytes()).map_err(|e| {
            SignalError::new(
                SignalErrorKind::Internal,
                format!("failed to write secret {key}"),
            )
            .with_source(e)
        })?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let path = self.secret_path(key)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SignalError::new(
                SignalErrorKind::NotFound,
                format!("secret {key} not found"),
            )),
            Err(e) => Err(SignalError::new(
                SignalErrorKind::Internal,
                format!("failed to delete secret {key}"),
            )
            .with_source(e)),
        }
    }

    fn rotate(&self, key: &str) -> Result<SecretString> {
        let _path = self.secret_path(key)?;
        let previous = self.get(key)?;
        let rotated = SecretString::from(uuid::Uuid::new_v4().to_string());
        write_secret_file(&self.secret_path(key)?, rotated.expose_secret().as_bytes()).map_err(
            |e| {
                SignalError::new(
                    SignalErrorKind::Internal,
                    format!("failed to rotate secret {key}"),
                )
                .with_source(e)
            },
        )?;
        Ok(previous)
    }
}

/// Composite store that layers multiple [`SecretStore`] implementations.
///
/// `get` tries each store in order and returns the first successful lookup.
/// Mutating operations (`put`, `delete`, `rotate`) try each store in order and
/// succeed with the first store that accepts the operation.
#[derive(Clone)]
pub struct CompositeSecretStore {
    stores: Vec<Arc<dyn SecretStore>>,
}

impl std::fmt::Debug for CompositeSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeSecretStore")
            .field("stores", &self.stores.len())
            .finish()
    }
}

impl CompositeSecretStore {
    /// Creates a composite store from an ordered list of stores.
    pub fn new(stores: Vec<Arc<dyn SecretStore>>) -> Self {
        Self { stores }
    }
}

impl SecretStore for CompositeSecretStore {
    fn get(&self, key: &str) -> Result<SecretString> {
        let mut last_err = None;
        for store in &self.stores {
            match store.get(key) {
                Ok(value) => return Ok(value),
                Err(err) => last_err = Some(err),
            }
        }
        Err(last_err
            .unwrap_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found")))
    }

    fn put(&self, key: &str, value: SecretString) -> Result<()> {
        let mut last_err = None;
        for store in &self.stores {
            match store.put(key, value.clone()) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == SignalErrorKind::Unsupported => last_err = Some(err),
                Err(err) => return Err(err),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            SignalError::new(
                SignalErrorKind::Unsupported,
                "no writable secret store accepted the operation",
            )
        }))
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut last_err = None;
        for store in &self.stores {
            match store.delete(key) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == SignalErrorKind::NotFound => last_err = Some(err),
                Err(err) if err.kind() == SignalErrorKind::Unsupported => continue,
                Err(err) => return Err(err),
            }
        }
        Err(last_err
            .unwrap_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found")))
    }

    fn rotate(&self, key: &str) -> Result<SecretString> {
        for store in &self.stores {
            match store.rotate(key) {
                Ok(value) => return Ok(value),
                Err(err) if err.kind() == SignalErrorKind::NotFound => continue,
                Err(err) if err.kind() == SignalErrorKind::Unsupported => continue,
                Err(err) => return Err(err),
            }
        }
        Err(SignalError::new(
            SignalErrorKind::NotFound,
            "secret not found",
        ))
    }
}

/// Creates `dir` and sets its permissions so only the owner can access it.
#[cfg(unix)]
fn create_secret_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_secret_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Writes `contents` to `path` with owner-only permissions.
#[cfg(unix)]
fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn in_memory_round_trip() -> Result<()> {
        let store = InMemorySecretStore::new();
        store.put("sig.test", SecretString::from("hunter2"))?;
        let value = store.get("sig.test")?;
        assert_eq!(value.expose_secret(), "hunter2");
        Ok(())
    }

    #[test]
    fn in_memory_delete_and_rotate() -> Result<()> {
        let store = InMemorySecretStore::new();
        store.put("k", SecretString::from("old"))?;
        let previous = store.rotate("k")?;
        assert_eq!(previous.expose_secret(), "old");
        let rotated = store.get("k")?;
        assert_ne!(rotated.expose_secret(), "old");

        store.delete("k")?;
        assert!(store.get("k").is_err());
        Ok(())
    }

    #[test]
    fn file_store_round_trip() -> Result<()> {
        let dir = tempfile::tempdir().map_err(|e| {
            SignalError::new(SignalErrorKind::Internal, "failed to create temp dir").with_source(e)
        })?;
        let store = FileSecretStore::new(dir.path());
        store.put("k", SecretString::from("v"))?;
        let value = store.get("k")?;
        assert_eq!(value.expose_secret(), "v");
        Ok(())
    }

    #[test]
    fn file_store_rejects_path_traversal() -> Result<()> {
        let dir = tempfile::tempdir().map_err(|e| {
            SignalError::new(SignalErrorKind::Internal, "failed to create temp dir").with_source(e)
        })?;
        let store = FileSecretStore::new(dir.path());
        assert!(store.get("../etc/passwd").is_err());
        Ok(())
    }

    #[test]
    fn env_store_maps_keys() -> Result<()> {
        let store = EnvSecretStore::new();
        assert_eq!(store.env_key("sig.test"), "CHEETAH_SECRET_SIG_TEST");

        let path_store = EnvSecretStore::with_prefix("");
        let value = path_store.get("path")?;
        let expected = std::env::var("PATH").unwrap_or_default();
        assert!(!expected.is_empty());
        assert_eq!(value.expose_secret(), expected);
        Ok(())
    }

    #[test]
    fn composite_prefers_first_source() -> Result<()> {
        let first = Arc::new(InMemorySecretStore::new());
        let second = Arc::new(InMemorySecretStore::new());
        first.put("k", SecretString::from("first"))?;
        second.put("k", SecretString::from("second"))?;

        let composite = CompositeSecretStore::new(vec![first, second]);
        let value = composite.get("k")?;
        assert_eq!(value.expose_secret(), "first");
        Ok(())
    }
}

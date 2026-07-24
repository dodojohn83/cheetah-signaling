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

/// Maximum bytes read from a single secret file.  128 KiB covers PEM certificate
/// chains and API keys while preventing a misconfigured mount from causing OOM.
const MAX_SECRET_FILE_BYTES: usize = 128 * 1024;

/// Maximum byte length of a secret key name. This prevents a misconfigured or
/// untrusted caller from constructing an arbitrarily large environment variable
/// name or file path while normalizing the key.
const MAX_SECRET_KEY_BYTES: usize = 256;

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
            .cloned()
            .ok_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found"))?;
        let rotated = SecretString::from(uuid::Uuid::new_v4().to_string());
        guard.insert(key.to_string(), rotated);
        Ok(previous)
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

    fn env_key(&self, key: &str) -> Result<String> {
        if key.len() > MAX_SECRET_KEY_BYTES {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "secret key exceeds maximum length",
            ));
        }
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
        Ok(format!("{}{}", self.prefix, normalized))
    }
}

impl SecretStore for EnvSecretStore {
    fn get(&self, key: &str) -> Result<SecretString> {
        let env_key = self.env_key(key)?;
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
        if key.len() > MAX_SECRET_KEY_BYTES {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "secret key exceeds maximum length",
            ));
        }
        if key.is_empty()
            || key.chars().any(std::path::is_separator)
            || key.split(['/', '\\']).any(|c| c == ".." || c == ".")
        {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "secret key contains path separators or traversal components",
            ));
        }
        Ok(self.base_dir.join(key))
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, key: &str) -> Result<SecretString> {
        let path = self.secret_path(key)?;
        match read_secret_file(&path) {
            Ok(value) => Ok(SecretString::from(value)),
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
        #[cfg(not(unix))]
        {
            let _ = (key, value);
            return Err(SignalError::new(
                SignalErrorKind::Unsupported,
                "file secret store requires Unix for owner-only permissions",
            ));
        }
        #[cfg(unix)]
        {
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
        #[cfg(not(unix))]
        {
            let _ = key;
            return Err(SignalError::new(
                SignalErrorKind::Unsupported,
                "file secret store requires Unix for owner-only permissions",
            ));
        }
        #[cfg(unix)]
        {
            let _path = self.secret_path(key)?;
            let previous = self.get(key)?;
            let rotated = SecretString::from(uuid::Uuid::new_v4().to_string());
            write_secret_file(&self.secret_path(key)?, rotated.expose_secret().as_bytes())
                .map_err(|e| {
                    SignalError::new(
                        SignalErrorKind::Internal,
                        format!("failed to rotate secret {key}"),
                    )
                    .with_source(e)
                })?;
            Ok(previous)
        }
    }
}

/// Composite store that layers multiple [`SecretStore`] implementations.
///
/// `get` tries each store in order and returns the first successful lookup.
/// `put` and `rotate` succeed with the first store that accepts the operation.
/// `delete` removes the key from every layered store that holds it, so that no
/// readable copy remains once the call returns `Ok`.
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
        fn error_priority(err: &SignalError) -> u8 {
            if err.is_retryable() {
                2
            } else if matches!(
                err.kind(),
                SignalErrorKind::NotFound | SignalErrorKind::Unsupported
            ) {
                0
            } else {
                1
            }
        }

        let mut selected = None;
        for store in &self.stores {
            match store.get(key) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if selected
                        .as_ref()
                        .is_none_or(|s: &SignalError| error_priority(&err) > error_priority(s))
                    {
                        selected = Some(err);
                    }
                }
            }
        }
        Err(selected
            .unwrap_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found")))
    }

    fn put(&self, key: &str, value: SecretString) -> Result<()> {
        let mut accepted = false;
        let mut last_err = None;
        for store in &self.stores {
            match store.put(key, value.clone()) {
                Ok(()) => {
                    accepted = true;
                    break;
                }
                Err(err) if err.kind() == SignalErrorKind::Unsupported => last_err = Some(err),
                Err(err) => return Err(err),
            }
        }
        if !accepted {
            return Err(last_err.unwrap_or_else(|| {
                SignalError::new(
                    SignalErrorKind::Unsupported,
                    "no writable secret store accepted the operation",
                )
            }));
        }

        // A higher-priority read-only layer may still shadow the value. Verify
        // the effective read value matches what was written.
        let effective = self.get(key)?;
        if effective.expose_secret() == value.expose_secret() {
            Ok(())
        } else {
            Err(SignalError::new(
                SignalErrorKind::Unsupported,
                "secret value shadowed by a higher-priority layer",
            ))
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut deleted = false;
        let mut not_found_err = None;
        for store in &self.stores {
            match store.delete(key) {
                Ok(()) => deleted = true,
                Err(err) if err.kind() == SignalErrorKind::NotFound => not_found_err = Some(err),
                Err(err) if err.kind() == SignalErrorKind::Unsupported => continue,
                Err(err) => return Err(err),
            }
        }

        // A read-only layer may still expose the key. Verify the actual state
        // before deciding what to report.
        match self.get(key) {
            Ok(_) => Err(SignalError::new(
                SignalErrorKind::Unsupported,
                "secret remains readable in a read-only layer after delete",
            )),
            Err(err) if err.kind() == SignalErrorKind::NotFound => {
                if deleted {
                    Ok(())
                } else if let Some(err) = not_found_err {
                    Err(err)
                } else {
                    Err(SignalError::new(
                        SignalErrorKind::Unsupported,
                        "no writable secret store accepted the operation",
                    ))
                }
            }
            Err(err) => Err(err),
        }
    }

    fn rotate(&self, key: &str) -> Result<SecretString> {
        let mut accepted_idx = None;
        let mut previous = None;
        for (i, store) in self.stores.iter().enumerate() {
            match store.rotate(key) {
                Ok(prev) => {
                    previous = Some(prev);
                    accepted_idx = Some(i);
                    break;
                }
                Err(err) if err.kind() == SignalErrorKind::NotFound => continue,
                Err(err) if err.kind() == SignalErrorKind::Unsupported => continue,
                Err(err) => return Err(err),
            }
        }
        let (Some(idx), Some(previous)) = (accepted_idx, previous) else {
            return Err(SignalError::new(
                SignalErrorKind::NotFound,
                "secret not found",
            ));
        };

        // A higher-priority read-only layer may still shadow the rotated value.
        // Verify the effective read value matches the value in the layer we
        // just rotated.
        let rotated = self.stores[idx].get(key)?;
        let effective = self.get(key)?;
        if effective.expose_secret() == rotated.expose_secret() {
            Ok(previous)
        } else {
            Err(SignalError::new(
                SignalErrorKind::Unsupported,
                "rotated secret shadowed by a higher-priority layer",
            ))
        }
    }
}

/// Creates `dir` and any missing ancestors with owner-only (`0o700`) permissions.
/// Existing directories are left untouched, so shared parent directories are not
/// modified. Any newly created directories receive `0o700` at creation time.
#[cfg(unix)]
fn create_secret_dir(dir: &Path) -> std::io::Result<()> {
    use std::fs::DirBuilder;
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = DirBuilder::new();
    builder.recursive(true).mode(0o700);
    match builder.create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// Reads `path` without following symlinks and enforces [`MAX_SECRET_FILE_BYTES`].
fn read_secret_file(path: &Path) -> std::io::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        read_limited(file)
    }
    #[cfg(not(unix))]
    {
        let file = std::fs::File::open(path)?;
        read_limited(file)
    }
}

/// Reads at most [`MAX_SECRET_FILE_BYTES`] from `reader` and returns its UTF-8 contents.
fn read_limited<R: std::io::Read>(reader: R) -> std::io::Result<String> {
    use std::io::{Error, ErrorKind, Read};

    let mut reader = reader.take((MAX_SECRET_FILE_BYTES as u64) + 1);
    let mut buf = Vec::with_capacity(MAX_SECRET_FILE_BYTES);
    reader.read_to_end(&mut buf)?;
    if buf.len() > MAX_SECRET_FILE_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("secret file exceeds {MAX_SECRET_FILE_BYTES} bytes"),
        ));
    }
    String::from_utf8(buf).map_err(|e| Error::new(ErrorKind::InvalidData, e.utf8_error()))
}

/// Writes `contents` to `path` with owner-only permissions and without following symlinks.
#[cfg(unix)]
fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(contents)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
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
    #[cfg(unix)]
    fn file_store_rejects_oversized_secret() -> Result<()> {
        let dir = tempfile::tempdir().map_err(|e| {
            SignalError::new(SignalErrorKind::Internal, "failed to create temp dir").with_source(e)
        })?;
        let store = FileSecretStore::new(dir.path());
        let path = dir.path().join("k");
        let oversized = vec![b'x'; MAX_SECRET_FILE_BYTES + 1];
        std::fs::write(&path, &oversized).map_err(|e| {
            SignalError::new(SignalErrorKind::Internal, "failed to write temp file").with_source(e)
        })?;
        assert!(store.get("k").is_err());
        Ok(())
    }

    #[test]
    #[cfg(unix)]
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
        assert_eq!(store.env_key("sig.test")?, "CHEETAH_SECRET_SIG_TEST");

        let path_store = EnvSecretStore::with_prefix("");
        let value = path_store.get("path")?;
        let expected = std::env::var("PATH").unwrap_or_default();
        assert!(!expected.is_empty());
        assert_eq!(value.expose_secret(), expected);
        Ok(())
    }

    #[test]
    fn env_store_rejects_oversized_key() {
        let store = EnvSecretStore::new();
        let oversized = "a".repeat(MAX_SECRET_KEY_BYTES + 1);
        assert!(store.get(&oversized).is_err());
    }

    #[test]
    fn file_store_rejects_oversized_key() -> Result<()> {
        let dir = tempfile::tempdir().map_err(|e| {
            SignalError::new(SignalErrorKind::Internal, "failed to create temp dir").with_source(e)
        })?;
        let store = FileSecretStore::new(dir.path());
        let oversized = "a".repeat(MAX_SECRET_KEY_BYTES + 1);
        assert!(store.get(&oversized).is_err());
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

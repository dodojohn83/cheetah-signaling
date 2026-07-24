//! Assembly adapter for the GB28181 module.
//!
//! Process assembly (the `apps/` layer) is only responsible for dependency
//! injection and lifecycle. All GB28181 business mapping — translating
//! configuration into a validated domain config, resolving the digest secret
//! from the [`SecretStore`], and wiring the per-device credential provider —
//! lives here so that `apps/cheetah-signaling/src/assembly.rs` never encodes
//! protocol business rules.
//!
//! The adapter is Sans-I/O: it constructs a [`Gb28181Access`] state machine but
//! never binds a socket or spawns a task. A protocol driver
//! (`cheetah-gb28181-driver-tokio`) executes the resulting machine.

use crate::access::Gb28181Access;
use crate::config::{AuthPolicy, Gb28181DomainConfig};
use crate::error::AccessError;
use crate::ports::{CredentialError, CredentialProvider};
use crate::types::DeviceId;
use cheetah_domain::CompatibilityProfile;
use cheetah_signal_types::{SecretStore, SignalErrorKind, clamp_str};
use secrecy::{ExposeSecret, SecretSlice, SecretString};
use std::sync::Arc;

/// Minimum accepted digest secret length in bytes.
const MIN_DIGEST_SECRET_BYTES: usize = 32;
/// Maximum byte length of a secret reference used in assembly error diagnostics.
const MAX_ASSEMBLY_SECRET_REF_BYTES: usize = 256;

/// Errors returned while assembling a GB28181 access machine from settings.
///
/// Messages are safe to surface in startup logs: they never contain secret
/// material, only the reference key that failed to resolve.
#[derive(Debug, thiserror::Error)]
pub enum GbAssemblyError {
    /// The digest secret reference was not configured.
    #[error("gb28181 digest secret reference is required")]
    MissingDigestSecretRef,
    /// The digest secret could not be read from the secret store.
    #[error("failed to resolve gb28181 digest secret ({0})")]
    DigestSecretUnavailable(String),
    /// The resolved digest secret was not valid hex.
    #[error("gb28181 digest secret ({0}) must be hex-encoded")]
    DigestSecretNotHex(String),
    /// The decoded digest secret was shorter than the minimum length.
    #[error("gb28181 digest secret ({0}) must decode to at least 32 bytes")]
    DigestSecretTooShort(String),
    /// The configured digest secret reference is too long to be a safe lookup key.
    #[error("gb28181 digest secret reference is too long ({0})")]
    DigestSecretRefTooLong(String),
    /// The domain configuration was rejected by the module.
    #[error("invalid gb28181 domain config: {0}")]
    DomainConfig(#[from] AccessError),
}

impl GbAssemblyError {
    fn digest_secret_unavailable(reference: &str) -> Self {
        Self::DigestSecretUnavailable(clamp_str(reference, MAX_ASSEMBLY_SECRET_REF_BYTES))
    }

    fn digest_secret_not_hex(reference: &str) -> Self {
        Self::DigestSecretNotHex(clamp_str(reference, MAX_ASSEMBLY_SECRET_REF_BYTES))
    }

    fn digest_secret_too_short(reference: &str) -> Self {
        Self::DigestSecretTooShort(clamp_str(reference, MAX_ASSEMBLY_SECRET_REF_BYTES))
    }

    fn digest_secret_ref_too_long(reference: &str) -> Self {
        Self::DigestSecretRefTooLong(clamp_str(reference, MAX_ASSEMBLY_SECRET_REF_BYTES))
    }
}

/// Declarative settings for constructing a GB28181 access machine.
///
/// This is the boundary type between process configuration and GB28181 business
/// logic. Assembly maps its transport-neutral configuration into this struct;
/// the adapter applies the protocol defaults and validation.
#[derive(Clone, Debug)]
pub struct GbAccessSettings {
    domain_id: String,
    realm: String,
    digest_secret_ref: String,
    challenge_optional: bool,
    device_password_ref: Option<String>,
    compatibility_profile: Option<CompatibilityProfile>,
}

impl GbAccessSettings {
    /// Default logical domain id used when configuration leaves it empty.
    pub const DEFAULT_DOMAIN_ID: &'static str = "34020000002000000001";

    /// Creates settings for a single GB28181 domain.
    ///
    /// When `domain_id` is empty the [`DEFAULT_DOMAIN_ID`](Self::DEFAULT_DOMAIN_ID)
    /// is used. The realm defaults to the resolved domain id, matching the
    /// GB28181 convention where the realm equals the SIP domain.
    pub fn new(domain_id: impl AsRef<str>, digest_secret_ref: impl Into<String>) -> Self {
        let domain_id = domain_id.as_ref();
        let domain_id = if domain_id.is_empty() {
            Self::DEFAULT_DOMAIN_ID.to_string()
        } else {
            domain_id.to_string()
        };
        Self {
            realm: domain_id.clone(),
            domain_id,
            digest_secret_ref: digest_secret_ref.into(),
            challenge_optional: false,
            device_password_ref: None,
            compatibility_profile: None,
        }
    }

    /// Overrides the SIP realm advertised in digest challenges.
    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = realm.into();
        self
    }

    /// Enables or disables the development-only "challenge optional" policy.
    ///
    /// When `true` the machine sends a digest challenge but still accepts
    /// unauthenticated REGISTER. The production default is `false`.
    pub fn with_challenge_optional(mut self, challenge_optional: bool) -> Self {
        self.challenge_optional = challenge_optional;
        self
    }

    /// Sets the optional per-device password reference template.
    ///
    /// The template may contain the `{device_id}` placeholder, which is
    /// substituted with the GB28181 device id before the secret store lookup.
    pub fn with_device_password_ref(mut self, reference: Option<String>) -> Self {
        self.device_password_ref = reference;
        self
    }

    /// Sets the optional compatibility profile applied to the access machine.
    pub fn with_compatibility_profile(mut self, profile: Option<CompatibilityProfile>) -> Self {
        self.compatibility_profile = profile;
        self
    }

    /// Resolved logical domain id.
    pub fn domain_id(&self) -> &str {
        &self.domain_id
    }

    /// Resolved SIP realm.
    pub fn realm(&self) -> &str {
        &self.realm
    }

    /// Whether the development-only challenge-optional policy is enabled.
    pub fn challenge_optional(&self) -> bool {
        self.challenge_optional
    }

    /// Authentication policy derived from [`challenge_optional`](Self::challenge_optional).
    pub fn auth_policy(&self) -> AuthPolicy {
        if self.challenge_optional {
            AuthPolicy::ChallengeOptional
        } else {
            AuthPolicy::Required
        }
    }

    /// Optional compatibility profile for this listener.
    pub fn compatibility_profile(&self) -> Option<&CompatibilityProfile> {
        self.compatibility_profile.as_ref()
    }
}

/// Credential provider backed by a [`SecretStore`].
///
/// The configured reference template may contain the `{device_id}` placeholder,
/// which is replaced with the GB28181 device identifier before the secret store
/// is queried. Missing optional secrets return `Ok(None)` so the domain can fall
/// back to challenge-based authentication when enabled; backend failures are
/// returned as [`CredentialError::Backend`].
#[derive(Clone)]
pub struct SecretStoreCredentialProvider {
    store: Arc<dyn SecretStore>,
    ref_template: Option<String>,
}

impl std::fmt::Debug for SecretStoreCredentialProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretStoreCredentialProvider")
            .field("ref_template", &self.ref_template)
            .finish_non_exhaustive()
    }
}

impl SecretStoreCredentialProvider {
    /// Creates a provider from a secret store and optional reference template.
    pub fn new(store: Arc<dyn SecretStore>, ref_template: Option<String>) -> Self {
        Self {
            store,
            ref_template: ref_template.map(|t| clamp_str(&t, MAX_ASSEMBLY_SECRET_REF_BYTES)),
        }
    }
}

impl CredentialProvider for SecretStoreCredentialProvider {
    fn password_for(&self, device_id: &DeviceId) -> Result<Option<SecretString>, CredentialError> {
        let Some(template) = self.ref_template.as_ref() else {
            return Ok(None);
        };
        let key = template.replace("{device_id}", device_id.as_ref());
        match self.store.get(&key) {
            Ok(secret) => Ok(Some(secret)),
            Err(e) if e.kind() == SignalErrorKind::NotFound => Ok(None),
            Err(e) => Err(CredentialError::Backend(e.to_string())),
        }
    }
}

/// Builds a validated [`Gb28181DomainConfig`] from settings and a secret store.
///
/// Resolves the hex-encoded digest secret by reference, decodes it, and enforces
/// the minimum secret length before handing it to the domain config.
pub fn build_domain_config(
    settings: &GbAccessSettings,
    secret_store: &Arc<dyn SecretStore>,
) -> Result<Gb28181DomainConfig, GbAssemblyError> {
    let digest_secret = resolve_digest_secret(secret_store.as_ref(), &settings.digest_secret_ref)?;
    let config = Gb28181DomainConfig::new(&settings.domain_id, &settings.realm, digest_secret)?
        .with_auth_policy(settings.auth_policy());
    let config = if let Some(profile) = settings.compatibility_profile() {
        config.with_compatibility(profile.clone())
    } else {
        config
    };
    Ok(config)
}

/// Builds a ready-to-drive GB28181 access state machine.
///
/// This is the single entry point assembly uses to obtain a
/// [`Gb28181Access`]: it resolves the digest secret, applies the authentication
/// policy, and wires the [`SecretStoreCredentialProvider`]. Assembly then hands
/// the returned machine to a transport driver.
pub fn build_access(
    settings: &GbAccessSettings,
    secret_store: &Arc<dyn SecretStore>,
) -> Result<Gb28181Access<SecretStoreCredentialProvider>, GbAssemblyError> {
    let domain_config = build_domain_config(settings, secret_store)?;
    let credential_provider = SecretStoreCredentialProvider::new(
        secret_store.clone(),
        settings.device_password_ref.clone(),
    );
    let access = Gb28181Access::new(domain_config, credential_provider)?;
    Ok(access)
}

/// Resolves and validates the digest secret referenced by `reference`.
fn resolve_digest_secret(
    store: &dyn SecretStore,
    reference: &str,
) -> Result<SecretSlice<u8>, GbAssemblyError> {
    if reference.is_empty() {
        return Err(GbAssemblyError::MissingDigestSecretRef);
    }
    if reference.len() > MAX_ASSEMBLY_SECRET_REF_BYTES {
        return Err(GbAssemblyError::digest_secret_ref_too_long(reference));
    }
    let secret = store
        .get(reference)
        .map_err(|_| GbAssemblyError::digest_secret_unavailable(reference))?;
    let bytes = hex::decode(secret.expose_secret().trim())
        .map_err(|_| GbAssemblyError::digest_secret_not_hex(reference))?;
    if bytes.len() < MIN_DIGEST_SECRET_BYTES {
        return Err(GbAssemblyError::digest_secret_too_short(reference));
    }
    Ok(SecretSlice::from(bytes))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use cheetah_signal_types::{Result as SignalResult, SignalError};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MapSecretStore {
        entries: Mutex<HashMap<String, String>>,
    }

    impl MapSecretStore {
        fn arc(entries: &[(&str, &str)]) -> Arc<dyn SecretStore> {
            let map = entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect();
            Arc::new(Self {
                entries: Mutex::new(map),
            })
        }
    }

    impl SecretStore for MapSecretStore {
        fn get(&self, key: &str) -> SignalResult<SecretString> {
            match self.entries.lock().unwrap().get(key) {
                Some(v) => Ok(SecretString::from(v.clone())),
                None => Err(SignalError::new(
                    SignalErrorKind::NotFound,
                    "missing secret",
                )),
            }
        }
        fn put(&self, key: &str, value: SecretString) -> SignalResult<()> {
            self.entries
                .lock()
                .unwrap()
                .insert(key.to_string(), value.expose_secret().to_string());
            Ok(())
        }
        fn delete(&self, key: &str) -> SignalResult<()> {
            self.entries.lock().unwrap().remove(key);
            Ok(())
        }
        fn rotate(&self, key: &str) -> SignalResult<SecretString> {
            self.get(key)
        }
    }

    fn valid_secret_hex() -> String {
        "ab".repeat(32)
    }

    #[test]
    fn settings_apply_default_domain_and_realm() {
        let settings = GbAccessSettings::new("", "secret://digest");
        assert_eq!(settings.domain_id(), GbAccessSettings::DEFAULT_DOMAIN_ID);
        assert_eq!(settings.realm(), GbAccessSettings::DEFAULT_DOMAIN_ID);
        assert_eq!(settings.auth_policy(), AuthPolicy::Required);
    }

    #[test]
    fn challenge_optional_maps_to_policy() {
        let settings =
            GbAccessSettings::new("3402000000", "secret://digest").with_challenge_optional(true);
        assert_eq!(settings.auth_policy(), AuthPolicy::ChallengeOptional);
    }

    #[test]
    fn build_access_succeeds_with_valid_secret() {
        let store = MapSecretStore::arc(&[("secret://digest", &valid_secret_hex())]);
        let settings = GbAccessSettings::new("3402000000", "secret://digest");
        let access = build_access(&settings, &store);
        assert!(access.is_ok());
    }

    #[test]
    fn missing_secret_reference_is_rejected() {
        let store = MapSecretStore::arc(&[]);
        let settings = GbAccessSettings::new("3402000000", "");
        assert!(matches!(
            build_access(&settings, &store),
            Err(GbAssemblyError::MissingDigestSecretRef)
        ));
    }

    #[test]
    fn unavailable_secret_is_reported_by_reference() {
        let store = MapSecretStore::arc(&[]);
        let settings = GbAccessSettings::new("3402000000", "secret://absent");
        match build_access(&settings, &store) {
            Err(GbAssemblyError::DigestSecretUnavailable(reference)) => {
                assert_eq!(reference, "secret://absent");
            }
            other => panic!("expected DigestSecretUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn non_hex_secret_is_rejected() {
        let store = MapSecretStore::arc(&[("secret://digest", "not-hex-value")]);
        let settings = GbAccessSettings::new("3402000000", "secret://digest");
        assert!(matches!(
            build_access(&settings, &store),
            Err(GbAssemblyError::DigestSecretNotHex(_))
        ));
    }

    #[test]
    fn short_secret_is_rejected() {
        let store = MapSecretStore::arc(&[("secret://digest", "abcd")]);
        let settings = GbAccessSettings::new("3402000000", "secret://digest");
        assert!(matches!(
            build_access(&settings, &store),
            Err(GbAssemblyError::DigestSecretTooShort(_))
        ));
    }

    #[test]
    fn unavailable_secret_error_clamps_oversized_reference() {
        let store = MapSecretStore::arc(&[]);
        let long_ref = "secret://".to_string() + &"x".repeat(1024);
        let settings = GbAccessSettings::new("3402000000", &long_ref);
        match build_access(&settings, &store) {
            Err(GbAssemblyError::DigestSecretRefTooLong(reference)) => {
                assert_eq!(reference.len(), MAX_ASSEMBLY_SECRET_REF_BYTES);
                assert!(reference.is_char_boundary(reference.len()));
            }
            other => panic!("expected DigestSecretRefTooLong, got {other:?}"),
        }
    }

    #[test]
    fn build_domain_config_rejects_oversized_realm() {
        let store = MapSecretStore::arc(&[("secret://digest", &valid_secret_hex())]);
        let long_realm = "r".repeat(128);
        let settings =
            GbAccessSettings::new("3402000000", "secret://digest").with_realm(long_realm);
        assert!(matches!(
            build_domain_config(&settings, &store),
            Err(GbAssemblyError::DomainConfig(AccessError::Internal(_)))
        ));
    }

    #[test]
    fn credential_provider_substitutes_device_id_and_maps_not_found() {
        let store = MapSecretStore::arc(&[("gb/dev/34020000001320000001", "pw")]);
        let provider =
            SecretStoreCredentialProvider::new(store, Some("gb/dev/{device_id}".to_string()));
        let known = DeviceId::new("34020000001320000001").expect("valid device id");
        let unknown = DeviceId::new("34020000001320000009").expect("valid device id");
        assert!(provider.password_for(&known).unwrap().is_some());
        assert!(provider.password_for(&unknown).unwrap().is_none());
    }

    #[test]
    fn credential_provider_without_template_returns_none() {
        let store = MapSecretStore::arc(&[]);
        let provider = SecretStoreCredentialProvider::new(store, None);
        let device = DeviceId::new("34020000001320000001").expect("valid device id");
        assert!(provider.password_for(&device).unwrap().is_none());
    }

    #[test]
    fn build_access_rejects_oversized_digest_secret_ref() {
        let store = MapSecretStore::arc(&[]);
        let long_ref = "x".repeat(1024);
        let settings = GbAccessSettings::new("3402000000", &long_ref);
        assert!(matches!(
            build_access(&settings, &store),
            Err(GbAssemblyError::DigestSecretRefTooLong(_))
        ));
    }

    #[test]
    fn credential_provider_clamps_oversized_password_ref_template() {
        let store = MapSecretStore::arc(&[]);
        let long_template = "gb/dev/{device_id}".to_string() + &"x".repeat(1024);
        let provider = SecretStoreCredentialProvider::new(store, Some(long_template));
        let device = DeviceId::new("34020000001320000001").expect("valid device id");
        assert!(provider.password_for(&device).unwrap().is_none());
    }
}

//! GB28181 domain and module configuration.

use secrecy::SecretString;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

/// Per-realm GB28181 configuration.
#[derive(Clone)]
pub struct Gb28181Config {
    /// Tenant that owns devices under this realm.
    pub tenant_id: cheetah_signal_types::TenantId,
    /// SIP realm (usually the domain ID).
    pub realm: String,
    /// Listen endpoint for UDP/TCP traffic.
    pub listen_endpoint: SocketAddr,
    /// SIP standard version string advertised in responses.
    pub standard_version: String,
    /// Character set policy for XML bodies.
    pub charset_policy: CharsetPolicy,
    /// Authentication policy.
    pub auth_policy: AuthPolicy,
    /// Heartbeat timeout in seconds.
    pub heartbeat_timeout_seconds: u64,
    /// Maximum number of catalog items per page.
    pub catalog_page_size: u32,
    /// XML parser/generator limits.
    pub xml_limits: XmlLimits,
    /// Compatibility profiles for known vendors.
    pub compatibility_profiles: Vec<CompatibilityProfile>,
    /// Default display name for unnamed devices.
    pub default_device_name: String,
}

/// XML body character-set handling.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CharsetPolicy {
    /// Strict UTF-8.
    #[default]
    Utf8,
    /// Allow GB2312/GBK encoded bodies and transcode to UTF-8.
    Gb2312,
    /// Allow GBK.
    Gbk,
}

/// Digest authentication policy.
#[derive(Clone)]
pub struct AuthPolicy {
    /// Server secret used to sign nonces.
    pub server_secret: SecretString,
    /// Whether to allow MD5 for legacy devices.
    pub allow_md5: bool,
    /// Nonce TTL in seconds.
    pub nonce_ttl_seconds: u64,
    /// Realm/tenant password lookup.
    pub password_lookup: Arc<dyn PasswordLookup>,
}

impl std::fmt::Debug for AuthPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthPolicy")
            .field("server_secret", &"[REDACTED]")
            .field("allow_md5", &self.allow_md5)
            .field("nonce_ttl_seconds", &self.nonce_ttl_seconds)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Gb28181Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181Config")
            .field("tenant_id", &self.tenant_id)
            .field("realm", &self.realm)
            .field("listen_endpoint", &self.listen_endpoint)
            .field("standard_version", &self.standard_version)
            .field("charset_policy", &self.charset_policy)
            .field("auth_policy", &self.auth_policy)
            .field("heartbeat_timeout_seconds", &self.heartbeat_timeout_seconds)
            .field("catalog_page_size", &self.catalog_page_size)
            .field("xml_limits", &self.xml_limits)
            .field("compatibility_profiles", &self.compatibility_profiles.len())
            .field("default_device_name", &self.default_device_name)
            .finish()
    }
}

/// Looks up the password for a device identity.
pub trait PasswordLookup: Send + Sync {
    /// Returns the configured password for `device_id` in `realm`, if any.
    fn password_for(&self, device_id: &str, realm: &str) -> Option<SecretString>;
}

/// In-memory password lookup for tests.
#[derive(Clone, Default)]
pub struct InMemoryPasswordLookup {
    passwords: HashMap<String, SecretString>,
}

impl std::fmt::Debug for InMemoryPasswordLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryPasswordLookup")
            .field("count", &self.passwords.len())
            .finish()
    }
}

impl InMemoryPasswordLookup {
    /// Creates an empty lookup.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a password for a device identity.
    pub fn insert(&mut self, device_id: impl Into<String>, password: impl Into<String>) {
        self.passwords
            .insert(device_id.into(), SecretString::from(password.into()));
    }
}

impl PasswordLookup for InMemoryPasswordLookup {
    fn password_for(&self, device_id: &str, _realm: &str) -> Option<SecretString> {
        self.passwords.get(device_id).cloned()
    }
}

/// Limits for the XML codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XmlLimits {
    /// Maximum XML body size in bytes.
    pub max_body_bytes: usize,
    /// Maximum element depth.
    pub max_depth: usize,
    /// Maximum text node length in bytes.
    pub max_text_len: usize,
    /// Maximum number of items in a list (e.g. catalog items).
    pub max_list_items: usize,
    /// Maximum length of an attribute value in bytes.
    pub max_attr_len: usize,
    /// Maximum total attribute count per element.
    pub max_attrs_per_element: usize,
}

impl Default for XmlLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 2_097_152,
            max_depth: 64,
            max_text_len: 65_536,
            max_list_items: 200_000,
            max_attr_len: 4_096,
            max_attrs_per_element: 64,
        }
    }
}

/// A vendor/model/firmware compatibility workaround.
#[derive(Clone, Debug)]
pub struct CompatibilityProfile {
    /// Human-readable profile name.
    pub name: String,
    /// Matching conditions.
    pub conditions: MatchConditions,
    /// Enabled behavior changes.
    pub behavior: WorkaroundBehavior,
    /// Risk note.
    pub risk: String,
    /// Optional captured sample reference.
    pub test_sample: Option<String>,
    /// Version when the profile should be re-evaluated.
    pub review_version: String,
}

/// Conditions for enabling a compatibility profile.
#[derive(Clone, Debug, Default)]
pub struct MatchConditions {
    /// User-Agent substring.
    pub user_agent: Option<String>,
    /// Device ID prefix.
    pub device_id_prefix: Option<String>,
    /// Realm substring.
    pub realm: Option<String>,
}

/// Behavior changes applied when a profile matches.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkaroundBehavior {
    /// Ignore XML declaration charset and force UTF-8.
    pub ignore_xml_charset: bool,
    /// Allow missing or fixed SN in MESSAGE responses.
    pub allow_fixed_sn: bool,
    /// Use the source address instead of the Contact header for responses.
    pub use_source_address: bool,
    /// Strip whitespace from XML element names.
    pub trim_xml_tags: bool,
}

/// Builds a `Gb28181Config` for tests and drivers.
#[derive(Clone)]
pub struct Gb28181ConfigBuilder {
    tenant_id: cheetah_signal_types::TenantId,
    realm: String,
    listen_endpoint: SocketAddr,
    standard_version: String,
    charset_policy: CharsetPolicy,
    allow_md5: bool,
    server_secret: SecretString,
    nonce_ttl_seconds: u64,
    password_lookup: Arc<dyn PasswordLookup>,
    heartbeat_timeout_seconds: u64,
    catalog_page_size: u32,
    xml_limits: XmlLimits,
    default_device_name: String,
    compatibility_profiles: Vec<CompatibilityProfile>,
}

impl std::fmt::Debug for Gb28181ConfigBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181ConfigBuilder")
            .field("tenant_id", &self.tenant_id)
            .field("realm", &self.realm)
            .field("listen_endpoint", &self.listen_endpoint)
            .finish_non_exhaustive()
    }
}

impl Gb28181ConfigBuilder {
    /// Starts a builder with sensible defaults.
    pub fn new(
        tenant_id: cheetah_signal_types::TenantId,
        realm: impl Into<String>,
        listen_endpoint: SocketAddr,
    ) -> Self {
        Self {
            tenant_id,
            realm: realm.into(),
            listen_endpoint,
            standard_version: "GB/T 28181-2016".into(),
            charset_policy: CharsetPolicy::Utf8,
            allow_md5: false,
            server_secret: SecretString::from(
                "this-is-a-very-long-server-secret-used-for-testing-only-do-not-use".to_string(),
            ),
            nonce_ttl_seconds: 300,
            password_lookup: Arc::new(InMemoryPasswordLookup::default()),
            heartbeat_timeout_seconds: 60,
            catalog_page_size: 100,
            xml_limits: XmlLimits::default(),
            default_device_name: "GB28181 device".into(),
            compatibility_profiles: Vec::new(),
        }
    }

    /// Sets whether MD5 is allowed for legacy devices.
    pub fn allow_md5(mut self, allow: bool) -> Self {
        self.allow_md5 = allow;
        self
    }

    /// Sets the server secret used to sign digest nonces.
    pub fn server_secret(mut self, secret: impl Into<String>) -> Self {
        self.server_secret = SecretString::from(secret.into());
        self
    }

    /// Sets the password lookup.
    pub fn password_lookup(mut self, lookup: Arc<dyn PasswordLookup>) -> Self {
        self.password_lookup = lookup;
        self
    }

    /// Sets the catalog page size.
    pub fn catalog_page_size(mut self, size: u32) -> Self {
        self.catalog_page_size = size;
        self
    }

    /// Builds the configuration.
    pub fn build(self) -> Gb28181Config {
        Gb28181Config {
            tenant_id: self.tenant_id,
            realm: self.realm,
            listen_endpoint: self.listen_endpoint,
            standard_version: self.standard_version,
            charset_policy: self.charset_policy,
            auth_policy: AuthPolicy {
                server_secret: self.server_secret,
                allow_md5: self.allow_md5,
                nonce_ttl_seconds: self.nonce_ttl_seconds,
                password_lookup: self.password_lookup,
            },
            heartbeat_timeout_seconds: self.heartbeat_timeout_seconds,
            catalog_page_size: self.catalog_page_size,
            xml_limits: self.xml_limits,
            compatibility_profiles: self.compatibility_profiles,
            default_device_name: self.default_device_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    #[test]
    fn builder_produces_valid_config() {
        let tenant_id = cheetah_signal_types::TenantId::generate();
        let addr = SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 5060));
        let config = Gb28181ConfigBuilder::new(tenant_id, "3402000000", addr).build();
        assert_eq!(config.realm, "3402000000");
        assert_eq!(config.heartbeat_timeout_seconds, 60);
    }
}

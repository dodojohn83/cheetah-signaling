//! Validated GB28181 identity newtypes.

/// A GB28181 device identifier.
///
/// Currently accepts 1-64 ASCII digits to accommodate both strict 20-digit GB
/// IDs and transitional identifiers. The exact validation rules may be
/// tightened per domain policy in the future.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceId(String);

impl DeviceId {
    /// Maximum length for a device identifier.
    pub const MAX_LEN: usize = 64;

    /// Creates a device identifier from a raw string.
    ///
    /// Returns `None` if the value is empty, too long, or contains characters
    /// other than ASCII digits.
    pub fn new(id: impl AsRef<str>) -> Option<Self> {
        let id = id.as_ref();
        if id.is_empty() || id.len() > Self::MAX_LEN {
            return None;
        }
        if !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(Self(id.to_string()))
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for DeviceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A logical GB28181 domain identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DomainId(String);

impl DomainId {
    /// Maximum length for a domain identifier.
    pub const MAX_LEN: usize = 64;

    /// Creates a domain identifier from a raw string.
    ///
    /// Returns `None` if the value is empty, too long, or contains control or
    /// whitespace characters.
    pub fn new(id: impl AsRef<str>) -> Option<Self> {
        let id = id.as_ref();
        if id.is_empty()
            || id.len() > Self::MAX_LEN
            || id.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return None;
        }
        Some(Self(id.to_string()))
    }
}

impl std::fmt::Display for DomainId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for DomainId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

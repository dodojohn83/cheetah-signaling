//! Configurable safety limits for GB28181 XML parsing and generation.

/// Upper bounds used by the XML codec to reject adversarial or malformed
/// payloads before they consume unbounded memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlLimits {
    /// Maximum XML element nesting depth.
    pub max_depth: usize,
    /// Maximum accumulated text length inside a single element.
    pub max_text_len: usize,
    /// Maximum number of quick-xml events to process.
    pub max_events: usize,
    /// Maximum raw body size in bytes before decoding.
    pub max_body_bytes: usize,
    /// Maximum child elements per parent.
    pub max_children_per_element: usize,
    /// Maximum attributes on a single element.
    pub max_attributes_per_element: usize,
    /// Maximum total attributes across the whole document.
    pub max_total_attributes: usize,
    /// Maximum length of an element or attribute name.
    pub max_name_len: usize,
    /// Maximum unknown extension entries stored for a message.
    pub max_extension_entries: usize,
    /// Maximum length of an extension map key.
    pub max_extension_key_len: usize,
    /// Maximum length of an extension map value.
    pub max_extension_value_len: usize,
}

impl XmlLimits {
    /// Conservative defaults suitable for GB28181 MANSCDP/MANSRTSP messages.
    pub const fn default() -> Self {
        Self {
            max_depth: 32,
            max_text_len: 4096,
            max_events: 10_000,
            max_body_bytes: 65_536,
            max_children_per_element: 10_000,
            max_attributes_per_element: 64,
            max_total_attributes: 100_000,
            max_name_len: 256,
            max_extension_entries: 256,
            max_extension_key_len: 128,
            max_extension_value_len: 1024,
        }
    }

    /// Minimal limits for unit tests.
    pub const fn test() -> Self {
        Self {
            max_depth: 4,
            max_text_len: 64,
            max_events: 64,
            max_body_bytes: 1_024,
            max_children_per_element: 8,
            max_attributes_per_element: 4,
            max_total_attributes: 32,
            max_name_len: 32,
            max_extension_entries: 4,
            max_extension_key_len: 16,
            max_extension_value_len: 32,
        }
    }
}

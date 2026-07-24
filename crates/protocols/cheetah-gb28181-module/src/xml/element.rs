//! Generic, bounded XML element tree used by the GB28181 XML codec.

use crate::error::AccessError;
use std::collections::HashMap;

/// A generic XML element with attributes, text and child elements.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct XmlElement {
    /// Local element name.
    pub name: String,
    /// Attributes by local name.
    pub attributes: HashMap<String, String>,
    /// Concatenated text content of this element.
    pub text: String,
    /// Child elements in document order.
    pub children: Vec<XmlElement>,
}

impl XmlElement {
    /// Returns the first direct child with the given local name.
    pub fn child(&self, name: &str) -> Option<&XmlElement> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Returns the trimmed text of the first direct child with the given name.
    pub fn child_text(&self, name: &str) -> Option<String> {
        self.child(name).map(|c| c.text.trim().to_string())
    }

    /// Returns the non-empty trimmed text of the first direct child with the given name,
    /// or an `InvalidXml` error if it is missing or empty.
    pub fn require_child_text(&self, name: &str) -> Result<String, AccessError> {
        self.child_text(name)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| AccessError::invalid_xml(format!("missing {name}")))
    }

    /// Collects leaf children whose names are not in `known` into an extension
    /// map. Unknown children that themselves have children are skipped.
    pub fn extension_map(&self, known: &[&str]) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for child in &self.children {
            if known.contains(&child.name.as_str()) || !child.children.is_empty() {
                continue;
            }
            let value = child.text.trim();
            if !child.name.is_empty() && !value.is_empty() {
                map.insert(child.name.clone(), value.to_string());
            }
        }
        map
    }
}

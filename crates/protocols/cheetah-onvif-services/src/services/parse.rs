//! Shared XML parse helpers for ONVIF service responses.

use crate::config::ParserLimits;
use crate::error::OnvifServiceError;
use cheetah_onvif_core::OnvifError;

/// Tracks parser limits while scanning an ONVIF response.
pub(crate) struct ParseContext<'a> {
    limits: &'a ParserLimits,
    stack: Vec<String>,
    text: String,
    node_count: usize,
}

impl<'a> ParseContext<'a> {
    pub(crate) fn new(limits: &'a ParserLimits, input: &str) -> Result<Self, OnvifServiceError> {
        if input.len() > limits.max_input_bytes {
            return Err(limit_error(format!(
                "response exceeds {} bytes",
                limits.max_input_bytes
            )));
        }
        Ok(Self {
            limits,
            stack: Vec::new(),
            text: String::new(),
            node_count: 0,
        })
    }

    pub(crate) fn on_start(&mut self, name: String) -> Result<(), OnvifServiceError> {
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(limit_error(format!(
                "response exceeds {} nodes",
                self.limits.max_nodes
            )));
        }
        if self.stack.len().saturating_add(1) > self.limits.max_depth {
            return Err(limit_error(format!(
                "response exceeds {} nesting depth",
                self.limits.max_depth
            )));
        }
        self.stack.push(name);
        self.text.clear();
        Ok(())
    }

    pub(crate) fn on_empty(&mut self) -> Result<(), OnvifServiceError> {
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(limit_error(format!(
                "response exceeds {} nodes",
                self.limits.max_nodes
            )));
        }
        if self.stack.len().saturating_add(1) > self.limits.max_depth {
            return Err(limit_error(format!(
                "response exceeds {} nesting depth",
                self.limits.max_depth
            )));
        }
        Ok(())
    }

    pub(crate) fn append_text(&mut self, s: &str) -> Result<(), OnvifServiceError> {
        if self.text.len().saturating_add(s.len()) > self.limits.max_text_bytes {
            return Err(limit_error(format!(
                "response text exceeds {} bytes",
                self.limits.max_text_bytes
            )));
        }
        self.text.push_str(s);
        Ok(())
    }

    pub(crate) fn on_end(&mut self) -> String {
        std::mem::take(&mut self.text)
    }

    pub(crate) fn pop(&mut self) {
        self.stack.pop();
    }

    pub(crate) fn parent(&self) -> Option<&str> {
        let len = self.stack.len();
        if len >= 2 {
            self.stack.get(len - 2).map(|s| s.as_str())
        } else {
            None
        }
    }
}

pub(crate) fn local_name(name: &quick_xml::name::QName<'_>) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).to_string()
}

pub(crate) fn limit_error(message: impl Into<String>) -> OnvifServiceError {
    OnvifServiceError::Onvif(OnvifError::LimitExceeded(message.into()))
}

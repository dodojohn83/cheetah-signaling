//! Pagination support for list operations.

use crate::{
    UtcTimestamp,
    error::{Result, SignalError, SignalErrorKind},
};
use uuid::Uuid;

/// Default page size when not specified.
pub const DEFAULT_PAGE_SIZE: u32 = 20;
/// Maximum page size allowed.
pub const MAX_PAGE_SIZE: u32 = 1_000;

/// A request for a single page of results.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PageRequest {
    /// Opaque cursor for the next page.
    pub cursor: Option<String>,
    /// Maximum number of items to return.
    pub page_size: u32,
}

impl PageRequest {
    /// Creates a page request with a validated size.
    pub fn new(page_size: u32) -> Result<Self> {
        if page_size == 0 {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "page_size must be greater than zero",
            ));
        }
        if page_size > MAX_PAGE_SIZE {
            return Err(SignalError::new(
                SignalErrorKind::InvalidArgument,
                format!("page_size must not exceed {MAX_PAGE_SIZE}"),
            ));
        }
        Ok(Self {
            cursor: None,
            page_size,
        })
    }

    /// Sets the cursor.
    #[must_use]
    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    /// Returns the page size as a `usize` for indexing.
    pub fn page_size_as_usize(self) -> usize {
        self.page_size as usize
    }
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            cursor: None,
            page_size: DEFAULT_PAGE_SIZE,
        }
    }
}

/// A page of results.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Page<T> {
    /// Items in this page.
    pub items: Vec<T>,
    /// Cursor for the next page, if any.
    pub next_cursor: Option<String>,
    /// Total number of items, if known.
    pub total: Option<u64>,
}

impl<T> Page<T> {
    /// Creates a new page with the given items.
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            next_cursor: None,
            total: None,
        }
    }

    /// Sets the next cursor.
    #[must_use]
    pub fn with_next_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.next_cursor = Some(cursor.into());
        self
    }

    /// Sets the total count.
    #[must_use]
    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }

    /// Maps the items to another type.
    pub fn map<U, F>(self, f: F) -> Page<U>
    where
        F: FnMut(T) -> U,
    {
        Page {
            items: self.items.into_iter().map(f).collect(),
            next_cursor: self.next_cursor,
            total: self.total,
        }
    }
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
            total: None,
        }
    }
}

/// Opaque cursor used for stable pagination across list queries.
///
/// The cursor captures the `updated_at` and resource id of the last item on the
/// current page so the next page can continue with `WHERE (updated_at, id) > ...`.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ListCursor {
    /// RFC 3339 timestamp of the last item.
    pub updated_at: String,
    /// String form of the resource identifier.
    pub id: String,
}

impl ListCursor {
    /// Creates a new cursor from a timestamp and id.
    pub fn new(updated_at: UtcTimestamp, id: Uuid) -> Result<Self> {
        Ok(Self {
            updated_at: updated_at.to_rfc3339()?,
            id: id.to_string(),
        })
    }

    /// Encodes the cursor as a JSON string.
    pub fn encode(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| {
            SignalError::new(SignalErrorKind::Internal, "failed to encode list cursor")
                .with_source(e)
        })
    }

    /// Decodes a cursor from its JSON string form.
    pub fn decode(value: &str) -> Result<Self> {
        serde_json::from_str(value).map_err(|e| {
            SignalError::new(SignalErrorKind::InvalidArgument, "invalid list cursor").with_source(e)
        })
    }

    /// Returns the timestamp and UUID represented by this cursor.
    pub fn parse(&self) -> Result<(UtcTimestamp, Uuid)> {
        let ts = UtcTimestamp::parse_rfc3339(&self.updated_at)?;
        let id = Uuid::parse_str(&self.id).map_err(|e| {
            SignalError::new(SignalErrorKind::InvalidArgument, "invalid cursor id").with_source(e)
        })?;
        Ok((ts, id))
    }
}

//! Pagination support for list operations.

use crate::error::{Result, SignalError, SignalErrorKind};

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

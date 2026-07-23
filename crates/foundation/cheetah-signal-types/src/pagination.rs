//! Pagination support for list operations.

use crate::{
    UtcTimestamp,
    error::{Result, SignalError, SignalErrorKind},
};
use base64::Engine as _;
use sha2::Digest;
use uuid::Uuid;

/// Default page size when not specified.
pub const DEFAULT_PAGE_SIZE: u32 = 20;
/// Maximum page size allowed.
pub const MAX_PAGE_SIZE: u32 = 1_000;
/// Current cursor format version.
const CURSOR_VERSION: u64 = 1;

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

impl PageRequest {
    /// Returns the page size clamped to a valid, positive range.
    pub fn clamped_page_size(&self) -> u32 {
        self.page_size.clamp(1, MAX_PAGE_SIZE)
    }

    /// Returns `clamped_page_size + 1` as `i64` for SQL `LIMIT` clauses.
    pub fn limit_plus_one(&self) -> i64 {
        i64::from(self.clamped_page_size()).saturating_add(1)
    }

    /// Returns the clamped page size as `usize` for in-memory slicing.
    pub fn page_size_as_usize_clamped(&self) -> usize {
        self.clamped_page_size() as usize
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
/// It includes a version and a checksum so accidental corruption or cursor format
/// drift (for example after a server restart that changes the sort key) is
/// detected explicitly. The checksum is not a keyed MAC; cursors are opaque
/// continuation tokens and the tenant filter is enforced separately server-side.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ListCursor {
    /// Cursor format version; changes invalidate previously issued cursors.
    #[serde(default)]
    pub version: u64,
    /// RFC 3339 timestamp of the last item.
    pub updated_at: String,
    /// String form of the resource identifier.
    pub id: String,
    /// Checksum over version, updated_at and id.
    #[serde(default)]
    pub checksum: String,
}

impl ListCursor {
    /// Creates a new cursor from a timestamp and id.
    pub fn new(updated_at: UtcTimestamp, id: Uuid) -> Result<Self> {
        let updated_at = updated_at.to_rfc3339()?;
        let id = id.to_string();
        let mut cursor = Self {
            version: CURSOR_VERSION,
            updated_at,
            id,
            checksum: String::new(),
        };
        cursor.checksum = cursor.compute_checksum();
        Ok(cursor)
    }

    /// Encodes the cursor as an opaque JSON string.
    pub fn encode(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| {
            SignalError::new(SignalErrorKind::Internal, "failed to encode list cursor")
                .with_source(e)
        })
    }

    /// Decodes a cursor from its opaque string form and verifies version and checksum.
    pub fn decode(value: &str) -> Result<Self> {
        let cursor: Self = serde_json::from_str(value).map_err(|e| {
            SignalError::new(
                SignalErrorKind::CursorExpired,
                "cursor has expired or is malformed",
            )
            .with_source(e)
        })?;

        if cursor.version != CURSOR_VERSION {
            return Err(SignalError::new(
                SignalErrorKind::CursorExpired,
                "cursor has expired: format version mismatch",
            ));
        }

        let expected = cursor.compute_checksum();
        if cursor.checksum != expected {
            return Err(SignalError::new(
                SignalErrorKind::CursorExpired,
                "cursor has expired: checksum mismatch",
            ));
        }

        Ok(cursor)
    }

    /// Returns the timestamp and UUID represented by this cursor.
    pub fn parse(&self) -> Result<(UtcTimestamp, Uuid)> {
        let ts = UtcTimestamp::parse_rfc3339(&self.updated_at)?;
        let id = Uuid::parse_str(&self.id).map_err(|e| {
            SignalError::new(
                SignalErrorKind::CursorExpired,
                "cursor has expired: invalid id",
            )
            .with_source(e)
        })?;
        Ok((ts, id))
    }

    /// Computes a stable, keyless checksum over the cursor payload.
    fn compute_checksum(&self) -> String {
        let payload = format!("{}:{}:{}", self.version, self.updated_at, self.id);
        let digest = sha2::Sha256::digest(payload.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn list_cursor_round_trips_and_verifies_integrity() {
        let ts = UtcTimestamp::from_offset(time::OffsetDateTime::UNIX_EPOCH);
        let id = Uuid::from_u128(42);
        let cursor = ListCursor::new(ts, id).unwrap();
        let encoded = cursor.encode().unwrap();
        let decoded = ListCursor::decode(&encoded).unwrap();
        assert_eq!(cursor, decoded);
    }

    #[test]
    fn list_cursor_rejects_stale_version() {
        let ts = UtcTimestamp::from_offset(time::OffsetDateTime::UNIX_EPOCH);
        let id = Uuid::from_u128(42);
        let mut cursor = ListCursor::new(ts, id).unwrap();
        cursor.version = 0;
        cursor.checksum = cursor.compute_checksum();
        let encoded = cursor.encode().unwrap();
        let err = ListCursor::decode(&encoded).unwrap_err();
        assert_eq!(err.kind(), SignalErrorKind::CursorExpired);
    }

    #[test]
    fn list_cursor_rejects_corrupted_checksum() {
        let ts = UtcTimestamp::from_offset(time::OffsetDateTime::UNIX_EPOCH);
        let id = Uuid::from_u128(42);
        let mut cursor = ListCursor::new(ts, id).unwrap();
        cursor.id = Uuid::from_u128(43).to_string();
        let encoded = cursor.encode().unwrap();
        let err = ListCursor::decode(&encoded).unwrap_err();
        assert_eq!(err.kind(), SignalErrorKind::CursorExpired);
    }

    #[test]
    fn clamped_page_size_handles_zero_and_overflow() {
        let zero = PageRequest {
            cursor: None,
            page_size: 0,
        };
        assert_eq!(zero.clamped_page_size(), 1);
        assert_eq!(zero.page_size_as_usize_clamped(), 1);
        assert_eq!(zero.limit_plus_one(), 2);

        let huge = PageRequest {
            cursor: None,
            page_size: u32::MAX,
        };
        assert_eq!(huge.clamped_page_size(), MAX_PAGE_SIZE);
        assert_eq!(huge.page_size_as_usize_clamped(), MAX_PAGE_SIZE as usize);
        assert_eq!(huge.limit_plus_one(), i64::from(MAX_PAGE_SIZE) + 1);
    }
}

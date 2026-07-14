//! Paginated list helpers for the SQLite storage adapter.

pub(crate) type Db = ::sqlx::Sqlite;

include!("../../cheetah-storage-api/include/list_impl.rs");

//! Paginated list helpers for the PostgreSQL storage adapter.

pub(crate) type Db = ::sqlx::Postgres;

include!("../../cheetah-storage-api/include/list_impl.rs");

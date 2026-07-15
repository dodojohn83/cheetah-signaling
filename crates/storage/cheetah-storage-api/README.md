# cheetah-storage-api

Storage port traits for repository, outbox and migration.

## 职责

- 定义 repository port、`UnitOfWork`、`Outbox` 和 migration 抽象。
- 提供 `TenantId` 作用域、revision 条件和分页接口。
- 不暴露 SQL/数据库实现细节。

## 允许依赖

- `cheetah-signal-types`、`cheetah-device-domain`、`cheetah-runtime-api`（workspace）。
- `serde`、`thiserror`、`uuid`、`chrono`（类型）。

## 禁止依赖

- 具体数据库驱动：`sqlx`、`tokio-postgres`、`rusqlite` 等不得进入本 crate。
- `tokio`、`axum`、`tonic`、`async-nats`、`quick-xml`。

## feature

- `default`：port trait。
- `sqlite`/`postgres`（未来）：在实现 crate 中启用，不在本 crate。

## 公共入口

- `src/lib.rs`：公开 repository port trait（WP-05 起填充）。

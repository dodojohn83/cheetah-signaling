# cheetah-runtime-api

Public runtime ports used by application, storage and messaging adapters.

## 职责

- 定义 `Scheduler`、`Timer`、`Clock`、`IdGenerator` 等运行时抽象 trait。
- 提供 `Send` future 约束的异步接口，供上层依赖。

## 允许依赖

- `std::future`、`std::time`（纯 trait 定义）。
- `serde`、`thiserror`（错误/配置类型）。
- `cheetah-signal-types`（workspace）。

## 禁止依赖

- 具体运行时实现（`tokio`）不得进入本 crate。
- 禁止 `axum`、`tonic`、`sqlx`、`async-nats`、`quick-xml` 等传输/协议/存储依赖。

## feature

- `default`：trait ports。

## 公共入口

- `src/lib.rs`：公开 runtime port trait（WP-04 起填充）。

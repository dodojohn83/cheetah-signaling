# cheetah-signal-contracts

Versioned internal contracts and capability constants shared across crates.

## 职责

- 声明跨 crate 使用的常量、能力版本和事件 schema 顶层结构。
- 作为 generated proto 的 staging 区域（WP-05 引入 prost 生成代码）。

## 允许依赖

- `serde`、`thiserror`。
- `cheetah-signal-types`（workspace）。

## 禁止依赖

- 运行时/传输层依赖（`tokio`、`axum`、`tonic`、`sqlx`、`async-nats`、`quick-xml`）。

## feature

- `default`：核心合同。
- `proto`（未来）：包含 prost 生成代码。

## 公共入口

- `src/lib.rs`：公开常量与合同类型（WP-05 起填充）。

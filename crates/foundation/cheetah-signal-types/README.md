# cheetah-signal-types

Shared domain primitives and newtype identities for the signaling control plane.

## 职责

- 提供 `TenantId`、`DeviceId`、`ChannelId`、`SessionId`、`OperationId` 等受校验 newtype。
- 定义时间、租户、revision、协议身份等基础领域类型。
- 保证类型在序列化、错误码和日志中的稳定表达。

## 允许依赖

- `serde`、`serde_json`（序列化）。
- `uuid`（UUIDv7 身份生成）。
- `thiserror`（错误 enum）。
- `chrono`（UTC 时间类型，不调用 now）。

## 禁止依赖

- `tokio`、`axum`、`tonic`、`sqlx`、`async-nats`、`quick-xml` 及任何运行时/传输层依赖。
- 不得依赖 `cheetah-device-domain` 等上层领域 crate。

## feature

- `default`：核心类型。

## 公共入口

- `src/lib.rs`：公开 newtype 和基础常量（WP-03 起逐步填充）。

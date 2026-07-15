# cheetah-messaging-api

Bus, inbox/outbox and ownership ports.

## 职责

- 定义消息总线、收件箱/发件箱、ownership 端口和消费语义。
- 支持 at-least-once 投递 + 幂等消费的抽象。

## 允许依赖

- `cheetah-signal-types`、`cheetah-runtime-api`（workspace）。
- `serde`、`thiserror`、`uuid`。

## 禁止依赖

- 具体消息中间件：`async-nats`、`lapin` 等不得进入本 crate。
- `tokio`、`axum`、`tonic`、`sqlx`、`quick-xml`。

## feature

- `default`：messaging port。
- `nats`/`local`（未来）：在实现 crate 中启用。

## 公共入口

- `src/lib.rs`：公开 `Bus`、`Inbox`、`Outbox` trait（WP-05 起填充）。

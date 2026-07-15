# cheetah-api-http

Northbound REST/SSE/Webhook handlers and DTO mappers.

## 职责

- 提供 `/api/v1` REST 端点、SSE 订阅和 Webhook 投递。
- 将 HTTP DTO 映射为 application 命令和查询。
- 不直接调用数据库、NATS 或媒体 socket。

## 允许依赖

- `axum`（HTTP 框架）。
- `tokio`（runtime）。
- `serde`、`serde_json`、`thiserror`、`uuid`、`tracing`。
- `cheetah-signal-application`、`cheetah-signal-types`（workspace）。

## 禁止依赖

- 具体存储/消息/协议实现：`sqlx`、`async-nats`、`tonic`、`quick-xml`。
- 媒体 payload 处理逻辑。

## feature

- `default`：HTTP API。
- `sse`/`webhook`（未来）。

## 公共入口

- `src/lib.rs`：公开 router builder 和 DTO mappers（WP-07 起填充）。

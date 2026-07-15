# cheetah-media-client

Media node gRPC client, registry and scheduler ports.

## 职责

- 通过 `cheetah.media.v1` gRPC 契约与媒体节点通信。
- 实现媒体节点发现、负载均衡和调度请求。
- 将媒体编排请求从 application 层转换为媒体 API 调用。

## 允许依赖

- `tonic`、`prost`（gRPC 生成与调用）。
- `tokio`（异步 runtime 调用）。
- `rustls`（TLS）。
- `cheetah-signal-types`、`cheetah-signal-contracts`、`cheetah-runtime-api`（workspace）。

## 禁止依赖

- 业务 domain crate（`cheetah-device-domain`）。
- 存储/HTTP/NATS/XML 实现：`sqlx`、`async-nats`、`axum`、`quick-xml`（媒体请求经 gRPC，不解析媒体 payload）。

## feature

- `default`：gRPC 客户端与 registry。

## 公共入口

- `src/lib.rs`：公开 `MediaClient`、`MediaRegistry` 等（WP-06 起填充）。

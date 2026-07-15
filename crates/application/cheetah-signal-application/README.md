# cheetah-signal-application

Application services: command handlers, Operation, Saga and reconciler.

## 职责

- 实现用例：设备接入、命令分发、操作编排、Saga 协调和 reconciler。
- 协调 domain、storage、messaging、cluster 和 media 端口。
- 不直接访问数据库、NATS、HTTP 或协议 socket。

## 允许依赖

- `cheetah-device-domain`、`cheetah-runtime-api`、`cheetah-storage-api`、`cheetah-messaging-api`、`cheetah-ownership`、`cheetah-media-client`（workspace）。
- `serde`、`thiserror`、`uuid`、`tracing`。

## 禁止依赖

- 具体适配器：`axum`、`tonic`、`sqlx`、`async-nats`、`quick-xml`。
- 直接使用 `tokio` 调度器（通过 runtime port）。

## feature

- `default`：核心 application 服务。

## 公共入口

- `src/lib.rs`：公开 `command`、`operation`、`saga` 等模块（WP-05 起填充）。

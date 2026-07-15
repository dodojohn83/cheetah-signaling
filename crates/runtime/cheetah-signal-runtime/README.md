# cheetah-signal-runtime

Sharded worker, timer wheel and runtime scheduling.

## 职责

- 实现固定分片 worker、分层时间轮和任务队列。
- 提供 `tokio` 版本的 `Scheduler`/`Timer`/`Clock` 实现。
- 管理 session map、timer wheel 和并发边界。

## 允许依赖

- `tokio`（runtime 实现）。
- `tracing`。
- `cheetah-runtime-api`、`cheetah-signal-types`、`cheetah-signal-contracts`（workspace）。

## 禁止依赖

- 业务 domain crate（`cheetah-device-domain`、`cheetah-signal-application`）。
- 存储/传输具体实现：`sqlx`、`async-nats`、`axum`、`tonic`、`quick-xml`。

## feature

- `default`：tokio runtime 实现。

## 公共入口

- `src/lib.rs`：公开 `Worker`、`TimerWheel` 等实现（WP-05 起填充）。

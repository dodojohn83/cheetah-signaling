# cheetah-gb28181-core

Sans-I/O GB/T 28181 core state machine and codec.

## 职责

- 实现 GB/T 28181 SIP/SDP/RTP 协商的 Sans-I/O 状态机。
- 解析/生成 SIP、SDP、XML 消息，处理 register/catalog/invite/bye 等事务。
- 不执行任何网络 I/O。

## 允许依赖

- `serde`、`thiserror`、`uuid`。
- `chrono`（类型）。
- `cheetah-signal-types`、`cheetah-signal-contracts`（workspace）。
- `quick-xml` 仅用于 XML 解析（SIP/SDP 经专门解析器，未来评估）。

## 禁止依赖

- `tokio`、`axum`、`tonic`、`sqlx`、`async-nats` 等运行时/传输/存储依赖。
- 不得直接调用系统时间或随机函数。

## feature

- `default`：核心状态机。

## 公共入口

- `src/lib.rs`：公开 `GbCore`、`Input`、`Output` 等类型（WP-08 起填充）。

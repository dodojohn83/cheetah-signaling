# cheetah-device-domain

Device, channel, session and media binding domain aggregates.

## 职责

- 定义设备、通道、会话、媒体绑定等聚合根和领域事件。
- 实现设备生命周期、通道树、能力集和命令状态机。
- 保证所有修改通过验证方法并维护不变量。

## 允许依赖

- `cheetah-signal-types`、`cheetah-signal-contracts`、`cheetah-runtime-api`（workspace）。
- `serde`、`thiserror`、`uuid`。
- `chrono`（UTC 时间类型，仅作类型，不调用 now）。

## 禁止依赖

- 任何运行时/传输层：`tokio`、`axum`、`tonic`、`sqlx`、`async-nats`、`quick-xml`。
- 不得直接依赖 `cheetah-media-client` 等媒体实现。

## feature

- `default`：核心领域模型。

## 公共入口

- `src/lib.rs`：公开 `device`、`channel`、`session` 等聚合模块（WP-04 起填充）。

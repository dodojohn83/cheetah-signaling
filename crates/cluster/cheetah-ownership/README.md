# cheetah-ownership

Owner directory, epoch fencing and lease management.

## 职责

- 实现节点/分片 owner 目录和 epoch 租约管理。
- 提供 `OwnerEpoch` fencing 原语，防止过时命令和设备状态被处理。
- 可作为 cluster 适配器或本地内存实现。

## 允许依赖

- `cheetah-signal-types`、`cheetah-runtime-api`、`cheetah-messaging-api`（workspace）。
- `serde`、`thiserror`、`uuid`。

## 禁止依赖

- 具体集群中间件实现细节不得污染领域接口。
- `tokio` 仅允许在实现层，不允许出现在 port 定义中。
- 禁止 `sqlx`、`axum`、`tonic`、`quick-xml`。

## feature

- `default`：ownership 核心。
- `nats`/`memory`（未来）：不同 backend 实现。

## 公共入口

- `src/lib.rs`：公开 `OwnerDirectory`、`Lease` 等类型（WP-06 起填充）。

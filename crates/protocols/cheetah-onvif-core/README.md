# cheetah-onvif-core

Sans-I/O ONVIF core state machine and XML codec.

## 职责

- 实现 ONVIF 设备发现、能力、媒体配置、PTZ 等事务的 Sans-I/O 状态机。
- 解析/生成 SOAP/XML 消息，执行 URL/地址安全检查。
- 不执行网络 I/O。

## 允许依赖

- `serde`、`thiserror`、`uuid`。
- `chrono`（类型）。
- `quick-xml`（ONVIF XML 解析）。
- `cheetah-signal-types`、`cheetah-signal-contracts`（workspace）。

## 禁止依赖

- `tokio`、`axum`、`tonic`、`sqlx`、`async-nats` 等运行时/传输/存储依赖。
- 不得直接调用系统时间或随机函数。

## feature

- `default`：核心状态机。

## 公共入口

- `src/lib.rs`：公开 `OnvifCore`、`Input`、`Output` 等类型（WP-09 起填充）。

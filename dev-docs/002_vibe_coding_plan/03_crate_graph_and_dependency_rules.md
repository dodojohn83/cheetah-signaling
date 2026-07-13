# 03. Crate 图与依赖规则

## 1. Crate 清单

| 分组 | package | 职责 |
| --- | --- | --- |
| foundation | `cheetah-signal-types` | ID、时间、错误、分页、context |
| foundation | `cheetah-signal-contracts` | generated Proto/re-export，不含业务 |
| runtime | `cheetah-runtime-api`、`cheetah-runtime-tokio` | clock、spawn、cancel、有界原语 |
| domain | `cheetah-device-domain` | Device/Endpoint/Channel/Capability |
| domain | `cheetah-control-domain` | Operation、MediaBinding、命令、事件、ports |
| application | `cheetah-signal-application` | handler、Saga、reconciler、policy |
| storage | `cheetah-storage-api/sqlite/postgres` | repository 与 UoW |
| messaging | `cheetah-message-api/local/nats` | bus、outbox publisher、inbox |
| cluster | `cheetah-cluster-ownership/registry` | owner、node/media registry |
| media | `cheetah-media-client/scheduler` | gRPC adapter 和 placement |
| api | `cheetah-http-api/cheetah-grpc-api` | 北向与内部 adapter |
| plugin | `cheetah-plugin-sdk/host/testkit` | 进程插件协议 |
| protocol | `cheetah-gb28181-*`、`cheetah-onvif-*` | core/driver/module |
| app | `cheetah-signaling` | role 装配和生命周期 |

## 2. 依赖方向

`app → adapters/application → domain/ports → types`；protocol module 可依赖 application ports，driver 只依赖 core/runtime，core 只依赖 types/codec 基础。

禁止：domain→storage、protocol core→runtime、module→tokio net/sqlx、HTTP→具体 PostgreSQL、media client→协议 crate、plugin SDK→host internals。

## 3. 任务

- [ ] 为每个 crate 创建 `README`，写职责、允许依赖、禁止依赖和公共入口。
- [ ] 建立 architecture test：解析 `cargo metadata`，对禁止边强制失败。
- [ ] feature 只表达可选能力/后端，不用 feature 改变同一公共类型语义。
- [ ] SQLite/PostgreSQL、local/NATS 可同时编译用于 contract test；应用运行时恰好选择一个权威 storage 和 bus profile。
- [ ] 将 generated Proto 隔离，domain 通过 mapper 转换，不直接 type alias。
- [ ] 所有 adapter 构造器显式注入依赖，禁止全局 singleton/service locator。

## 4. 完成条件

- `cargo tree` 无环、无重复大版本 TLS/HTTP/Proto 栈。
- 关闭 cluster feature 时 edge 不链接 async-nats/PostgreSQL client。
- domain/core 的 `cargo tree` 不出现 tokio、axum、tonic、sqlx、async-nats。

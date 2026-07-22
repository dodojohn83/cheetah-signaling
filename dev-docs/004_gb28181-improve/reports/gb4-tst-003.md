# GB4-TST-003：Driver-Core-Module 与架构契约测试

- 任务：`GB4-TST-003`
- 状态：`Done`
- 日期：2026-07-22

## 1. 已完成的架构契约测试

在 `cheetah-gb28181-driver-tokio` 和 `cheetah-gb28181-module` 中已有
`tests/architecture.rs`：

- `driver-tokio` 生产依赖必须包含 `cheetah-gb28181-core`；
- `driver-tokio` 生产依赖禁止包含 `cheetah-gb28181-module`；
- `module` 生产依赖必须包含 `cheetah-gb28181-core`；
- `module` 生产依赖禁止包含 `tokio`、`cheetah-plugin-sdk`、`async-trait`。

本次在 workspace 级 `crates/testing/cheetah-architecture-test` 追加基于
`cargo metadata` 传递依赖的层次检查：

- `domain` 不依赖 tokio/axum/tonic/sqlx/async-nats/quick-xml（既有）；
- `*-core` 不依赖 tokio/socket2/reqwest/hyper/sqlx/async-nats/media-client（既有）；
- crate 依赖图无环（既有）；
- 新增：`*-driver-tokio` 不得传递依赖 `sqlx` 或 `cheetah-media-client`（AGENTS §2.2，
  driver 拥有 socket/timer 但不得触达业务仓储或媒体客户端）。

> 说明：曾尝试对 `*-application` 与 `*-module` 加通用禁止依赖断言，但与当前既有
> 架构不符（`cheetah-onvif-module` 合法使用 tokio；application 传递依赖 async-nats），
> 故不引入与现状冲突的失败断言，保留 driver 侧这一条真实不变量。

## 2. 新增契约测试 crate

新增 `crates/testing/cheetah-contract-tests`（`publish = false`，附 `README.md`），
补齐此前 Partial 报告中待补的 message / media 契约。确定性夹具，无 RTP/RTCP/媒体负载。

### 2.1 message（`tests/message.rs`）

- `encode_command`/`decode_command` 往返：断言 envelope meta 的
  message/tenant/correlation/owner-epoch/idempotency/operation 字段，并 `decoded == command`；
- `encode_event`/`decode_event` 往返：断言 aggregate_sequence 与 event_id，并 `decoded == event`；
- in-process bus（`InProcessMessageBus`）命令 FIFO 顺序；
- 至少一次 + 幂等消费：同一命令重复投递携带相同 message_id，按 id 去重后仅处理一次；
- 事件广播扇出到多个订阅者；无订阅者时 publish 为 no-op。

### 2.2 media（`tests/media_port.rs`）

针对 `InMemoryMediaPort` 的 `MediaPort` 契约：

- 预留确定性且 `contract_version=1`、instance epoch 非零；
- 重复预留返回 `Unavailable`；release 后可重新预留；
- 租户隔离：同一 binding id 在两租户下互不影响；`list_nodes` 按租户过滤；
- StartLive→Accepted 且出现在 `list_sessions`；StopMediaSession→Completed 后移除；
- 设备类命令（Ptz）经媒体端口返回 `InvalidArgument`。

## 3. repository 契约现状

repository 事务/outbox/revision/租户/inbox 幂等契约已由既有
`crates/testing/cheetah-storage-tests` 的 SQLite/PostgreSQL 共享 contract suite 覆盖
（`sqlite_contract_suite`、`postgres_contract_suite` 均通过），本任务不重复实现，仅在
message/media/架构层补齐缺口。

## 4. 验证命令

```bash
cargo test -p cheetah-contract-tests
cargo test -p cheetah-architecture-test
cargo test -p cheetah-storage-tests
cargo clippy --workspace --all-targets -- -D warnings
```

结果：message 6、media_port 6、architecture 4、storage sqlite/postgres 各 1，全部通过。

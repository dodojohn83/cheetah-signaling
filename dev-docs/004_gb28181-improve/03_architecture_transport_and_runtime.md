# 03. 架构、SIP Transport 与分片运行时

## 1. 目标

修复 GB28181 三段式依赖，建立唯一生产数据流，并将已有 SIP parser、transaction、dialog 和协议状态机装入支持 UDP/TCP、固定分片、分层时间轮和有界背压的运行时。

本阶段结束后，设备输入和 application command 必须通过同一 core 状态机；不得再存在 listener 路径与 plugin command 路径各自维护状态的情况。

## 2. 冻结分层

```text
apps/assembly
  -> GB application adapter / event sink
  -> cheetah-gb28181-module
  -> cheetah-gb28181-driver-tokio
  -> cheetah-gb28181-core + runtime/port foundation
```

允许依赖：

- core：signal-types/foundation、无 I/O 数据结构和纯算法；
- driver：core、runtime port、Tokio 网络/时间；
- module：core、领域/application 所需的下层 typed port；
- assembly：构造 driver、module mapper、SecretStore adapter、application handler 和 cancellation。

禁止依赖：

- driver → module/application/plugin host/SQL/NATS/media client；
- module → Tokio/Axum/Tonic/SQLx/async-nats/plugin SDK/media client；
- core → Tokio/socket/clock now/secret provider/quick-xml I/O adapter；
- assembly 承载目录合并、协议状态迁移或 vendor workaround。

## 3. Core 接口

将当前 module 内 wire 状态机迁入 core，并按职责保留三个小状态机：

```rust
GbAccessMachine
GbMediaMachine
GbCascadeMachine
```

每个状态机公开显式、不可执行 I/O 的类型：

```rust
AccessInput / AccessOutput
MediaInput / MediaOutput
CascadeInput / CascadeOutput
GbTimerId
GbTransportCommand
GbProtocolEvent
```

统一输入至少覆盖：

- `SipRequestReceived` / `SipResponseReceived`；
- `TransportConnected` / `TransportClosed`；
- `TimerFired`；
- `ApplicationCommand`；
- `CredentialResolved`；
- `MediaStepCompleted` / `MediaEventReceived`；
- `OwnershipChanged` / `CancelRequested`。

统一输出至少覆盖：

- `SendSip`；
- `CloseConnection`；
- `ArmTimer` / `CancelTimer`；
- `ResolveCredential`；
- `EmitProtocolEvent`；
- `RequestApplicationStep`；
- `CompleteCommand` / `FailCommand` / `UnknownCommandOutcome`。

core 不持有异步 CredentialProvider。需要凭据时输出 `ResolveCredential`，adapter 使用 SecretStore 和 deadline 查询，再将脱敏结果作为 input 返回。

## 4. 单一生产数据流

### 4.1 入站

```text
UDP datagram / TCP frame
  -> parser + listener/domain router
  -> shard key(tenant, protocol identity/transaction)
  -> bounded shard mailbox
  -> transaction/dialog/access machine
  -> SIP response + typed protocol event
  -> application adapter
```

### 4.2 出站命令

```text
Operation/Command + owner epoch
  -> GB command mapper
  -> target shard
  -> access/media/cascade machine
  -> client transaction
  -> UDP/TCP transport
  -> response/timer input
  -> OperationStep outcome
```

移除以下生产路径：

- `cheetah/gb28181` built-in plugin 的 `process_sip` JSON/hex 接口；
- `NoopCredentialProvider`；
- plugin capability 固定声明 bidirectional/RTP 而未实际支持命令；
- OwnerCommandHandler 将 Unsupported 记录为 Completed/unknown；
- listener 与 plugin 各自创建独立 `Gb28181Access`。

若插件框架需要展示内置协议能力，使用只读 capability adapter 查询实际 GB runtime，不让 plugin host 成为命令或协议状态所有者。

## 5. 固定分片运行时

- 使用现有 runtime abstraction 或新增下层 port，将 `(tenant_id, protocol_identity)` 稳定哈希到固定 shard。
- 每个 shard 单线程拥有其 registration、transaction、dialog、subscription 和轻量 session 状态；跨 shard 只通过 typed message。
- device 未活跃时状态可惰性卸载；权威业务状态由 repository/Operation 保存。
- timer 使用分层时间轮或等价有界 scheduler；禁止每设备 `tokio::sleep`。
- mailbox、每轮 batch、timer 数、active transaction、TCP connection、parser buffer 都有配置上限。
- shard 不得跨 `.await` 持有协议状态锁；I/O 输出交给 driver executor，结果再入 mailbox。
- shutdown 先撤销 readiness 和 admission，再停止新命令，drain mailbox/transaction 到 deadline，取消 timer 并关闭 socket。

## 6. UDP/TCP Driver

### 6.1 UDP

- listener 支持 IPv4/IPv6 与多个 bind 地址；
- datagram 超过上限直接拒绝并计数，不分配无界 buffer；
- transaction machine 决定重传/吸收重复，access handler 不重复产生事件；
- 发送目标来自 transaction/dialog route，不使用“当前输入 source”作为通用默认；
- admission 满时，对可响应请求返回 503 + bounded Retry-After；无法安全响应时丢弃并指标化。

### 6.2 TCP

- 每连接使用增量 parser 处理半包、粘包和多 message；
- 限制连接数、每来源连接数、read buffer、write queue、idle timeout 和 pending transaction；
- write queue 满时停止读取或关闭慢连接，不使用 unbounded channel；
- connection close 转为 core input，清理关联 route/timer，但不伪造设备业务离线；
- cancellation 和 shutdown 释放 socket、buffer、permit 和 timer。

TLS 不作为 GB v1 必选 transport；若配置声明 TLS 而未实现，启动返回稳定 Unsupported，不静默降级 TCP。

## 7. Transaction、Dialog 与路由

- server/client transaction 使用 Via branch + sent-by + method，并处理 ACK/CANCEL 特例；
- reliable transport 禁用 UDP retransmission timer，但保留 overall deadline；
- request duplicate 返回缓存响应，不重复认证、落库或发 application event；
- response 按 transaction key 进入对应 shard；无匹配 response 记录受限诊断后丢弃；
- dialog 保存 Call-ID、local/remote tag、route set、remote target、CSeq 和 generation；
- re-INVITE、BYE、INFO、SUBSCRIBE/NOTIFY 使用 dialog CSeq 和 target，不复用 REGISTER route；
- late 2xx、forked response、重复 ACK 和 out-of-order in-dialog request 有明确状态表。

## 8. Tenant、Domain 与 Endpoint 路由

新增配置：

```toml
[[gb28181.listeners]]
id = "tenant-a-gb"
tenant_id = "..."
local_device_id = "34020000002000000001"
realm = "3402000000"
domain = "3402000000"
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
digest_secret_ref = "secret://..."
device_credential_namespace = "gb28181/tenant-a/devices"
```

- listener ID、Request-URI/To domain 和 realm 必须唯一解析到 tenant；歧义或未配置 domain 返回 403/404，不回退 default tenant。
- edge 可显式配置一个 tenant 的单 listener；cluster 禁止隐式 `default_tenant_id`。
- 旧 `sip_port/sip_domain/default_tenant_id` 配置保留一个发布窗口，启动时转换为单 listener 并发出弃用日志；新旧同时配置视为错误。
- endpoint 分为 `observed_source`、`via_received_rport`、`contact_uri`、`advertised_endpoint` 和 `dialog_remote_target`。
- 只有认证 REGISTER、dialog target refresh 或显式 compatibility profile 能改变发送 route；普通 Keepalive/MESSAGE 不得直接改写 endpoint。

## 9. 背压策略

| 类别 | 队列满行为 |
| --- | --- |
| REGISTER/final SIP response | 优先保留；必要时 503/reject admission，不静默丢弃已接受事务 |
| Alarm/command result/media terminal event | 写入可恢复 inbox/outbox 或拒绝输入；禁止静默 drop |
| Keepalive/presence refresh | 同设备可合并为最新状态，并记录 coalesced metric |
| Catalog fragment | 按聚合容量拒绝整个 SN 或产生 Partial；不能只丢中间片不报告 |
| TCP outbound | stop-read/backpressure，超过 deadline 关闭连接 |
| UDP outbound | bounded batch；过载进入 transaction failure/UnknownOutcome |

## 10. 实施任务

- [x] `GB4-ARC-001`：移动 wire 状态机并移除 driver → module、module → Tokio/plugin SDK 依赖；更新三个 crate README。
- [x] `GB4-ARC-002`：删除双入口和 Noop/plugin command 生产路径，建立唯一 runtime handle（`Gb28181UdpDriver` 直接由 assembly 构造，插件 driver 路径与 `NoopCredentialProvider` 已移除）。
- [x] `GB4-ARC-003`：实现固定 shard、惰性 session、bounded mailbox 和 timer wheel，并暴露健康指标（`cheetah-runtime-api`/`cheetah-runtime-tokio`：`ShardRouter` 稳定哈希固定分片、actor 惰性创建 + `actor_idle_timeout_ms` 空闲卸载、`AdmissionController` 有界 `try_send`、单 worker 时间轮、`RuntimeMetrics`/`RuntimeMetricsSnapshot` 聚合健康指标（无高基数设备 label），并含空闲卸载与 10 万 timer 暂停时间测试）。
- [ ] `GB4-ARC-004`：将 assembly 中 GB 业务映射迁到 module/application adapter，assembly 只保留 DI/lifecycle。
- [x] `GB4-ARC-005`：拆分超过 800 行的生产源文件，并对超过 500 行文件给出拆分或保留理由。
  - 已拆分 GB 相关 `cascade/machine.rs`（原 824 行）：命令驱动的注册流程与共享 helper 留在 `machine.rs`（483 行），SIP 响应分发与 REGISTER/deregister/keepalive 响应处理（含 digest challenge/resend）移入新 `machine_response.rs`（361 行）。
  - 其余 >800 行文件均不属于 GB28181 改造范围，按 AGENTS 规范以理由保留、由各自模块的独立工单跟踪拆分，不在本 GB4 变更内做无关重构：`crates/domain/cheetah-domain/src/in_memory.rs`、`crates/domain/cheetah-domain/src/operation.rs`（领域测试替身与聚合逻辑）、`apps/cheetah-signaling/src/assembly.rs`（装配/生命周期，属 `GB4-ARC-004` 范围）、`crates/storage/cheetah-storage-sqlite/src/repository.rs`、`crates/storage/cheetah-storage-postgres/src/repository.rs`（共享 repository contract 实现）、`crates/cluster/cheetah-cluster-ownership/src/assignment.rs`、`crates/cluster/cheetah-cluster-ownership/src/rolling_upgrade.rs`、`crates/foundation/cheetah-signal-types/src/config.rs`、`crates/media/cheetah-media-scheduler/src/grpc.rs`、`tools/onvif-simulator/src/main.rs`。
- [ ] `GB4-SIP-001`：完成 UDP/TCP/IPv4/IPv6 driver contract，含 framing、连接上限、cancel 和 shutdown。
- [ ] `GB4-SIP-002`：接入 transaction/dialog，覆盖 retransmission、duplicate、late/out-of-order 和 deadline。
- [ ] `GB4-SIP-003`：完成 REGISTER/MESSAGE/INVITE/ACK/CANCEL/BYE/INFO/SUBSCRIBE/NOTIFY/OPTIONS method 路由。
- [ ] `GB4-SIP-004`：实现 credential resolution output/input、Digest replay/stale/algorithm/rate-limit 生产链路。
- [ ] `GB4-SIP-005`：实现 multi-listener/domain/realm/tenant 路由与旧配置兼容窗口。
- [ ] `GB4-SIP-006`：实现 endpoint route 模型、NAT/rport 策略和 source hijack regression。

## 11. 测试与退出门禁

- core 不依赖任何 I/O/runtime crate；driver 不依赖 module；module 不依赖 Tokio/plugin SDK。
- `cargo metadata` architecture test 和 `scripts/audit_architecture.py` 对 GB 路径无 violation/warning。
- UDP duplicate request、response loss、reorder、Timer A/B/D/E/F/K 全部使用 FakeClock 验证。
- TCP 每个 byte 边界切片、多个消息粘包、慢读写、连接中断、idle timeout 和队列满均有测试。
- 10 万和 100 万 timer item 使用暂停时间测试，不创建等量 task/sleep。
- listener/domain/realm tenant 越界、endpoint 漂移和 credential timeout 有安全失败测试。
- production assembly 中只能搜索到一个 GB runtime 实例和一个命令入口。


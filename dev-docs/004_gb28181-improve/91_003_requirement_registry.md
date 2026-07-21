# 91. 003 要求与 004 稳定任务 Registry

## 1. 用途

本文件保证 003 中与 GB28181 垂直链路相关的要求都有后续归属。它不修改 003 checkbox，也不把相关通用媒体/集群任务复制为 GB 私有实现。

## 2. 003 直接映射

| 003 Task | 当前状态 | 004/上游归属 | 关闭条件摘要 |
| --- | --- | --- | --- |
| GB-001 接入事件落库 | `Partial` | GB4-ACC-001..005、GB4-EVT-001..002 | ProtocolSession、tenant/owner、目录和所有事件进入事务/outbox |
| GB-002 认证与凭据 | `Partial` | GB4-SIP-004..006、GB4-SEC-001..002 | SecretStore、nonce/replay、生产 Required、无敏感日志 |
| GB-003 命令路由 | `Not Implemented` | GB4-CMD-001..003 | typed command 直达 GB runtime，正确 OperationStep outcome |
| GB-004 媒体会话 | `Partial` | GB4-MED-001..008 + 003 MED/WF | real MediaPort Saga、补偿、对账和真实媒体测试 |
| GB-005 级联 | `Partial` | GB4-CAS-001..006 | 平台模型、生产 transport、目录/订阅/bridge/owner |
| GB-006 兼容与互操作 | `Partial` | GB4-REF、GB4-COMP、GB4-SYS-003..004 | profile + fixture + 两类设备 + 平台报告 |
| GB-007 验收 | `Not Implemented` | GB4-TST、GB4-SYS | failure/chaos/capacity/soak/control-plane 证据 |

## 3. 003 通用依赖映射

| 003 类别 | 004 使用方式 | 不得重复实现 |
| --- | --- | --- |
| BAS architecture/toolchain | GB4-AUD/ARC/SYS 复验并补 GB-specific gate | 新建另一套 workspace/toolchain |
| ASM runtime/ownership | GB runtime 接入固定 shard、owner/inbox/lifecycle | GB 私有 owner 或每设备 task runtime |
| MED-C typed media contract | GB mapper 和 contract suite 的唯一媒体 wire | GB 私有 gRPC/JSON media API |
| MED-R registry/client/events | GB Saga 调用和 media callback fencing | 访问 media engine/stream manager |
| WF Operation/MediaSession/Binding | GB live/playback/download/talk workflow | GB Command 生命周期或私有 session 权威状态 |
| PROD API/cluster/security | additive typed command、tenant/RBAC/ops | handler 直达 driver 或 GB 私有 auth |
| SYS interop/perf/release | GB scenario 进入统一 system harness | 用 simulator 单测替代真实验收 |
| UP-MEDIA-P0 | real media contract 阻塞依赖 | signaling 内复制上游实现 |

## 4. 004 任务 ID 清单

### Audit / Reference

- `GB4-AUD-001..003`
- `GB4-REF-001..004`

### Architecture / SIP

- `GB4-ARC-001..005`
- `GB4-SIP-001..006`

### Access / Command / Event

- `GB4-ACC-001..005`
- `GB4-CMD-001..003`
- `GB4-EVT-001..002`

### Media

- `GB4-MED-001..008`

### Cascade / Compatibility

- `GB4-CAS-001..006`
- `GB4-COMP-001..004`

### Security / Operations

- `GB4-SEC-001..004`
- `GB4-OPS-001..005`

### Test / System

- `GB4-TST-001..004`
- `GB4-SYS-001..009`

总任务数：68。任务范围写在对应 Phase 文档中；本文件只负责唯一性和上游映射。

## 5. 状态更新规则

1. task checkbox 只在其所属 Phase 文档更新。
2. `Completed` 后必须在 `reports/` 存在同 ID 报告或明确的聚合报告链接。
3. `Blocked` 必须写明具体 003/upstream task、所缺接口和已完成的本地工作。
4. `Superseded` 必须列出新任务 ID，不能删除历史任务。
5. 同一任务不得在多个 Phase 文档重复定义 checkbox；其他文件只能链接。
6. 修改任务范围或公共契约时同步本 registry、README 关键路径和相关 Phase。

## 6. Registry 验收

- 所有 `GB4-*` 定义 ID 必须唯一且连续。
- 每个定义 ID 只对应一个 checkbox。
- README、Phase、Reference 和 reports 相对链接必须有效。
- GB-001..007 及其 MED/WF/ASM/PROD/SYS 依赖不得存在未归属项。
- checkbox 计数必须与本文件声明的 68 项一致。

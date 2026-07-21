# 01. 003 完成度与 GB28181 现状审计

## 1. 审计结论

`dev-docs/003_next_round_vibe_coding_plan` 尚未完成。当前仓库具有较完整的 GB28181 SIP/XML/SDP 基础类型、Digest、事务/dialog 单元测试，以及未装配的媒体和级联 Sans-I/O 状态机；但生产数据流只闭环了部分 UDP REGISTER/MESSAGE 接入，无法满足 003 的真实协议垂直链路与全局完成定义。

003 中可计数任务为 43 项勾选、132 项未勾选。Phase 02 和 Phase 06 虽已全部勾选，但报告中的完成声明与当前 GB28181 生产装配存在矛盾，因此按证据规则降级为 `Partial`。

## 2. 审计环境与实际验证

- signaling HEAD：`96c76efc9b6c5bdf4956ab6a4c100429d0e8e8da`（当前执行基线；后续基线变化时由 `GB4-AUD-003` 复验）
- Rust：`rustc 1.96.1`
- 当前工作区审计前无用户改动
- 当前环境缺少 `buf` 和 `cargo-nextest`，无法重跑完整提交门禁

已执行：

```bash
cargo fmt --all -- --check
cargo clippy \
  -p cheetah-gb28181-core \
  -p cheetah-gb28181-module \
  -p cheetah-gb28181-driver-tokio \
  -p cheetah-gb28181-simulator \
  --all-targets -- -D warnings
cargo test \
  -p cheetah-gb28181-core \
  -p cheetah-gb28181-module \
  -p cheetah-gb28181-driver-tokio \
  -p cheetah-gb28181-simulator
python3 scripts/audit_architecture.py
```

结果：

- fmt：通过；
- GB28181 定向 clippy：通过；
- GB28181 定向测试：301 passed，0 failed；
- architecture audit：失败，3 个 layer violation、3 个 forbidden dependency warning、2 个 production panic warning。

与 GB28181 直接相关的架构错误为：

```text
cheetah-gb28181-driver-tokio (layer 5)
  -> cheetah-gb28181-module (layer 4)
```

## 3. 003 Phase 状态

| Phase | Checkbox | 当前状态 | 证据与缺口 | 004 归属 |
| --- | ---: | --- | --- | --- |
| 00 completion audit | 0/7 | `Partial` | registry 已生成，但 003 未完成重新归属与保守关闭 | GB4-AUD |
| 01 baseline/CI | 0/25 | `Partial` | 工具链报告存在；当前架构检查失败，完整门禁无法在本环境复现 | GB4-AUD、GB4-ARC、GB4-SYS |
| 02 assembly | 30/30 | `Partial` | 基础装配已增加，但 GB 存在双入口、占位 plugin command 和事件日志路径 | GB4-ARC、GB4-ACC |
| 03 media contract/runtime | 0/45 | `Not Completed` | typed contract、真实节点、事件和 readiness 未完成 | 003 MED-C/MED-R、GB4-MED |
| 04 media workflow | 0/10 | `Not Completed` | 四模型 Saga 与协议媒体状态机未装配 | 003 WF、GB4-MED |
| 05 GB/ONVIF | 1/46 | `Partial` | 仅 GB production auth 默认 Required 已闭环 | GB4-SIP..GB4-CAS |
| 06 API/cluster/security | 12/12 | `Partial` | error matrix、interop、DLQ/ops 等报告仍有 Remaining | 003 PROD、GB4-OPS、GB4-SYS |
| 07 system/release | 无 checkbox | `Not Implemented` | 没有 real media、real device、三节点 chaos、百万在线或 72h soak 报告 | GB4-SYS |
| 全局 DoD | N/A | `Not Completed` | 真实 GB/媒体垂直闭环和发布证据均缺失 | 全部 |

## 4. 当前可复用资产

### 4.1 Core

- SIP datagram/stream parser、encoder、URI/header、Digest client/server；
- transaction key/timer、INVITE/non-INVITE transaction 和 dialog；
- SDP parser/encoder 和注入/边界测试；
- parser/SDP property test、golden 和 malformed regression。

这些能力可继续作为 wire core，但 transaction/dialog 尚未进入生产 UDP driver，不能据此关闭 transport 或命令链路。

### 4.2 Module

- REGISTER/Digest、保活和 in-memory registration table；
- Catalog、DeviceInfo、DeviceStatus、Alarm、MobilePosition、RecordInfo、DeviceControl response XML；
- live/playback/download/talk 的 INVITE/ACK/CANCEL/BYE/INFO 局部状态机；
- 级联注册、保活、目录、subscription/notify 和 bridge 局部状态机；
- 140 个 module 单元测试及 XML golden/property test。

这些状态机大多满足 Sans-I/O 形态，但位于错误层次，且 production 未实例化媒体和级联对象。

### 4.3 Simulator 与 fixture

- simulator 支持 UDP、Digest、Keepalive、Alarm、Catalog、INVITE 和 seeded fault；
- synthetic profile 名称包含 generic/dahua/hikvision；
- fixture 只有 4 组 SIP 和 3 组 XML golden，均带 MIT-0 provenance metadata。

simulator 当前为每设备 task/socket/timer 结构，不能用于百万设备结论；vendor profile 只有合成字段差异，没有来自真实设备的 workaround 证据。

## 5. 生产链路缺口

### 5.1 双入口与错误命令结果

assembly 同时存在：

1. `Gb28181UdpDriver` 真实 UDP listener，使用 SecretStore credential 和 Required auth；
2. 内置 `cheetah/gb28181` plugin，使用 `NoopCredentialProvider` 和 ChallengeOptional，仅处理 `process_sip`。

OwnerCommandHandler 将 PTZ、StartLive、StopMediaSession、StartPlayback、StartTalk、ControlPlayback 发给内置 plugin；plugin 对这些命令返回 Unsupported，handler 又把错误映射为 `ProcessedMessageStatus::Completed` 和 `outcome=unknown`。这不等价于业务成功，也不能作为安全的 UnknownOutcome 生命周期。

### 5.2 Driver 未执行 core transaction/dialog

生产 driver 只实现 UDP datagram：解析后直接调用单个 `Gb28181Access`，每秒 tick 扫描 registration table。它没有：

- TCP listener/framing/连接治理；
- 事务重传、响应缓存和 dialog 路由；
- 固定分片运行时或分层时间轮；
- 主动请求和命令发送路径；
- 多 listener/domain/tenant 路由；
- source、Via received/rport 与 Contact 的独立策略。

### 5.3 Event sink 未形成权威状态

当前 sink 使用单个 `default_tenant_id`；REGISTER/presence/catalog/device info/status 可调用 DeviceService，但 Alarm、MobilePosition、DeviceControlResponse、RecordInfo、media 和 cascade 事件只记录日志。队列满时 `try_send` 静默丢弃所有类别事件，且每次输入生成新 MessageId/CorrelationId，无法稳定去重重传。

注册、online 和 owner acquisition 未形成一个可恢复的事务/Saga，旧 owner/media epoch 也没有贯穿 GB event。

### 5.4 Media 与 cascade 仅存在局部状态机

生产代码没有构造 `Gb28181Media::new` 或 `Gb28181Cascade::new`。现有 media `Tick` 不执行超时，INFO response 未形成 OperationStep；现有 cascade 没有生产配置、持久化 owner、transport 或真实平台互操作。

### 5.5 文档和代码边界不一致

- driver README 声明不得依赖 module，但 Cargo 实际依赖 module；
- module README 声明不依赖 Tokio/plugin transport，但实际依赖两者；
- app assembly 包含大量 GB 目录/事件业务映射，不再是纯装配层；
- cascade machine 源文件超过 800 行，另有多个文件超过建议的 500 行。

## 6. 审计退出门禁

- [x] `GB4-AUD-001`：将本审计的命令结果写入当前 commit 的正式报告（见 [reports/gb4-aud-001.md](reports/gb4-aud-001.md)）。
- [x] `GB4-AUD-002`：逐项核对 [91 registry](91_003_requirement_registry.md)，不存在未归属的 003 GB/媒体/系统要求；新增 `scripts/verify_gb4_registry.py` 自动化核对。
- [x] `GB4-AUD-003`：当代码基线变化时重新运行 checkbox、生产装配、依赖图、占位实现和验证工具审计；复验命令记录于 [reports/gb4-aud-001.md](reports/gb4-aud-001.md)。
- 阶段门禁：所有“已完成”判断同时具有生产入口、故障语义、测试和报告，不再仅依据 checkbox 或代码量。

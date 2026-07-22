# GB4-TST-004：fixed-shard simulator 与 deterministic fault scenario DSL

- 任务：`GB4-TST-004`
- 状态：`Completed`
- 日期：2026-07-22
- 范围：`tools/gb28181-simulator`

## 1. 目标

按 `08_testing_interop_performance_and_release.md` 第 5/10 节，重构 GB28181
模拟器为**固定分片、确定性、可重复**的离散事件 harness，并提供声明式故障场景
DSL，用于信令面负载与韧性测试。模拟器只模拟 SIP/媒体控制事件，**不处理任何真实
RTP/RTCP/PS/TS/ES 媒体负载**。

## 2. 架构

模拟器从每设备一个 Tokio task/socket/timer 的旧实现，重构为单一 crate 库 +
CLI，核心模块如下：

| 模块 | 职责 |
| --- | --- |
| `scenario` | TOML 场景 DSL：profile、steps、faults 与语义校验 |
| `clock` | `VirtualClock` + `TimerWheel`（`(due_ms, seq)` 确定性排序） |
| `rng` | 基于 SHA-256 从 `(seed, label[, index])` 派生的稳定 RNG 流 |
| `wire` | endpoint、frame、SIP→`MessageClass` 分类、payload-free 语义摘要 |
| `fault` | 确定性故障引擎（每规则独立 RNG 流） |
| `transport` | UDP 整包解析 / TCP 流式重组（半包、粘包） |
| `device` | Sans-I/O 设备状态机（REGISTER/digest/keepalive/catalog/INVITE/BYE） |
| `platform` | Sans-I/O 平台 peer（challenge/accept、catalog、脚本命令、SIP 错误注入） |
| `harness` | 固定分片调度器 + `RunReport`（计数、资源、transcript hash） |
| `report` | 可复现运行报告（JSON 序列化） |

关键约束落实：

- **固定分片、惰性状态**：设备按 `index % shards` 固定归属分片，无每设备
  task/timer；
- **单一时间轮**：设备启动、保活、脚本步骤、投递全部进入同一 `TimerWheel`，
  启动/保活用 seed 均匀错峰；
- **确定性**：时间、ID、jitter、每个故障决策均由 master seed 派生；
- **契约保持**：编码/解析复用 `cheetah-gb28181-core`（`SipParser` /
  `encode_message`）与 `cheetah-gb28181-module` XML builder，未改动现有 golden
  fixture 与 parser 契约；
- **资源有界**：UDP 共享 socket 数、TCP 连接池、时间轮峰值事件数均记录于报告。

## 3. 故障场景 DSL

场景为单个 TOML 文件（示例见 `tools/gb28181-simulator/scenarios/`）。故障类型：

| kind | 效果 |
| --- | --- |
| `drop` | 丢弃该帧 |
| `delay` | 增加 `extra_ms` + 均匀 `jitter_ms` 延迟 |
| `reorder` | 将该帧回退至多 `window` 个投递槽 |
| `duplicate` | 额外投递一份字节相同副本 |
| `half_packet` | 将帧拆成两段（仅 TCP，触发流式重组） |
| `malformed` | 破坏帧使解析失败（不 panic） |
| `sip_error` | 平台对匹配请求返回 SIP 错误码 |

每条规则可按 `direction`（device→platform / platform→device / both）与
`target`（register/keepalive/catalog/media/message/any）选择目标。校验拒绝：
零 shard/device/duration、`rate` 超出 `[0,1]`、reorder window 为零、
`sip_error` code 超出 `[300,699]`、UDP 上使用 `half_packet`、步骤时间晚于
`duration_ms`。

## 4. 输出报告

每次运行输出 JSON `RunReport`，绑定 seed、scenario、transport、profile
（含 `synthetic_vendor` 标记）、message counts、fault counts、语义 outcomes、
资源使用与 **transcript hash**（对 payload-free 语义摘要的滚动 SHA-256），
同 seed+scenario 两次运行结果逐字节一致。

## 5. 验证

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --bins
```

全部通过。simulator crate 新增 28 个单元测试，覆盖：DSL 解析/校验、时间轮排序、
RNG 稳定性、六类故障计数、UDP/TCP 解析与半包重组、设备/平台状态机、固定分片
harness 的可复现性（同 seed 相同 hash、不同 seed 不同 hash）、丢包/SIP 错误
阻断注册、TCP 半包仍可重组且零解析错误。

示例场景实测（`scenarios/`）：

- `baseline.toml`（50 设备，UDP，无故障）：全部注册/保活/catalog/INVITE/BYE
  成功，`parse_errors = 0`，时间轮峰值事件有界；
- `faults-tcp.toml`（200 设备，TCP，七类故障）：全部故障计数非零，
  `parse_errors` 等于 malformed 计数，设备在丢包/503 下经重试仍全部注册，
  两次运行 transcript hash 一致。

## 6. 媒体边界与 provenance

- 仅生成 control-plane synthetic SDP，不产生/收发任何 RTP/RTCP/PS/TS/ES 负载，
  不绑定媒体端口；
- `synthetic_vendor` profile 为行为夹具，**不构成互操作证据**；
- 参考实现仅作 clean-room 行为参考，未复制任何源码（见
  `90_reference_provenance_and_license.md`）。

## 7. 备注

- 旧 CLI 的实时 UDP 多设备行为被确定性离散事件 harness 取代；`main.rs` 现输出
  JSON 报告，支持 `--scenario` 或少量 flag 快速冒烟；
- 真实设备/NVR 互操作与真实媒体验证属 `GB4-SYS-*` 阶段，不在本任务范围。

# Cheetah Signaling GB28181 完善与生产闭环开发计划

## 1. 文档定位

本目录是 [003 下一轮开发计划](../003_next_round_vibe_coding_plan/README.md) 的 GB28181 专项继任计划。003 保留为历史执行基线；本目录依据当前代码、生产装配、验证结果和参考实现重新判定 GB28181 完成度，并将未闭环内容转换为可独立验证的任务。

本轮不以增加孤立 parser、状态机或测试 skeleton 为完成目标，而是形成以下生产闭环：

```text
北向 API / 内部 Command
  -> Operation / Saga / owner fencing
  -> GB28181 module mapper
  -> 固定分片协议运行时
  -> SIP UDP/TCP driver
  -> 设备或上下级平台
  -> SIP/XML/SDP event
  -> application / repository / outbox
  -> MediaPort / MediaSession / MediaBinding
  -> Operation 可查询终态
```

审计冻结点：

- `cheetah-signaling`: `1ebfbe3ed1fd7ea33f0fd21030140a33e17ba0d1`
- `simple-media-server`: `bd68e28745a9863f68d6a496fc077d43b9bf99aa`
- 审计日期：2026-07-21

后续执行若基线变化，任务报告必须记录新 commit、差异范围以及重新验证结果。

## 2. 状态与证据规则

任务只允许使用以下状态：

| 状态 | 判定 |
| --- | --- |
| `Completed` | 行为进入生产装配，正常与失败语义有测试，要求的验证或真实联调报告通过 |
| `Partial` | 类型、局部实现或测试存在，但生产链路、故障语义或验收缺失 |
| `Not Implemented` | 没有实现，或生产入口仍返回占位 Unsupported/Unknown |
| `Blocked` | 已明确依赖的契约、上游交付或环境尚未满足，无法安全关闭 |
| `Superseded` | 被其他稳定任务 ID 替代，并给出替代关系和原因 |

文件存在、代码可编译、单元测试通过均不能单独证明 `Completed`。完成证据至少包括：

1. 生产装配入口和真实数据流；
2. 正常、失败、超时、取消、重复、过载和旧 epoch 行为；
3. 实际执行的命令、环境与结果；
4. 需要设备、平台或媒体节点时的可复现互操作报告；
5. 没有违反 Control Plane、六层依赖和协议三段式边界。

## 3. 执行规则

1. 按 Phase 00–07 推进；前一阶段退出门禁未满足时，依赖任务不得标为完成。
2. PR、commit、测试和报告必须引用 `GB4-*` 稳定任务 ID。
3. 先冻结 typed contract 和 contract test，再修改 adapter 和生产装配。
4. 003 的 checkbox 不回填；其 GB28181 当前状态由 [01 审计](01_003_completion_audit.md) 解释。
5. 与 `001` 冻结架构冲突时停止实现，先更新设计或提交 ADR。
6. fake、simulator、固定成功、日志型 sink 或 Unsupported adapter 不能作为生产闭环证据。
7. 媒体上游未满足时可以完成 core、mapper、fake contract 和失败路径，但真实媒体任务保持 `Blocked`。
8. 参考项目只用于行为分析和 clean-room 测试；许可证未确认的源码不得复制。
9. 每个任务完成后在 `reports/` 增加证据，内容遵循 [报告规范](reports/README.md)。

## 4. 阶段索引

| Phase | 文档 | 退出结果 |
| --- | --- | --- |
| 00 | [01](01_003_completion_audit.md)、[02](02_reference_implementation_analysis.md)、[91](91_003_requirement_registry.md) | 当前状态、参考基线和所有依赖有保守归属 |
| 01 | [03](03_architecture_transport_and_runtime.md) | 三段式依赖恢复、单一生产入口、UDP/TCP 与固定分片运行时闭环 |
| 02 | [04](04_device_access_commands_and_events.md) | 接入、认证、命令、目录和事件持久化闭环 |
| 03 | [05](05_media_operations_and_reconciliation.md) | live/playback/download/talk 与 MediaPort/Saga/对账闭环 |
| 04 | [06](06_cascade_subscription_and_compatibility.md) | 上下级平台、订阅、桥接和显式兼容 profile 闭环 |
| 05 | [07](07_security_observability_and_operations.md) | 安全、观测、限流、过载和运维恢复可验证 |
| 06–07 | [08](08_testing_interop_performance_and_release.md) | 系统互操作、chaos、百万设备、72 小时 soak 和发布报告通过 |
| Reference | [90](90_reference_provenance_and_license.md) | 来源、许可证、fixture 和禁止照搬边界可审计 |

## 5. 关键路径

```text
GB4-AUD / GB4-REF
  -> GB4-ARC
  -> GB4-SIP
  -> GB4-ACC / GB4-CMD / GB4-EVT
  -> [003 MED-C + MED-R + WF + UP-MEDIA-P0]
  -> GB4-MED
  -> GB4-CAS / GB4-COMP
  -> GB4-OPS
  -> GB4-SYS
```

架构、transport、接入和 simulator 可在媒体上游交付前推进；真实媒体、级联桥接和最终系统验收必须等待对应依赖完成。

## 6. 全局完成定义

- driver 只依赖 core/下层 port，module 不依赖 Tokio、plugin SDK、SQL、NATS 或媒体 client。
- 生产只存在一个 GB28181 协议入口，不再有 `process_sip` JSON/hex、Noop credential 或命令占位路径。
- GB/T 28181-2022 为主路径、2016 为显式兼容路径；所有厂商 workaround 由 profile 管理。
- REGISTER、保活、目录、查询、控制、报警、位置、媒体和级联均具有 tenant、owner epoch、幂等和恢复语义。
- live、playback、download、talk 经版本化 MediaPort 执行；信令进程不绑定或处理媒体 payload。
- UDP/TCP、IPv4/IPv6、重复、乱序、超时、取消、过载、重启和旧 epoch 行为具有确定测试。
- SQLite/PostgreSQL、edge/cluster、fake/real media 使用相同 contract suite。
- fmt、clippy、nextest、Buf、breaking、migration、deny 和架构检查通过。
- 至少两类真实设备或 NVR、一个上下级平台完成脱敏互操作报告。
- 三节点故障注入、100 万在线容量和 72 小时 soak 具有可复现报告。


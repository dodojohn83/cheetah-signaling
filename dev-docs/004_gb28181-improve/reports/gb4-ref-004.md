# GB4-REF-004：GB28181 互操作报告

- 任务：`GB4-REF-004`
- 状态：`Blocked`
- 日期：2026-07-21

## 1. 目标

说明参考实现中哪些行为来自真实设备、参考 peer 或 simulator，避免把 peer 间一致误称为标准一致。

## 2. 证据来源矩阵

| 行为 | 真实设备 | 参考 peer | Simulator | 标准条款 | 说明 |
| --- | --- | --- | --- | --- | --- |
| UDP REGISTER 401/200 流程 | Pending | wvp-GB28181-pro, sipsdk, GB28181.Solution | 已实现 | GB/T 28181-2022 第 5/6 章 | 参考 peer 与 synthetic fixture 一致；待真实设备验证 |
| TCP SIP 传输 | Pending | wvp-GB28181-pro, AKStream | 部分实现 | GB/T 28181-2022 附录 | TCP 粘包/半包由状态机处理，已在 core 设计，待真实网络验证 |
| Catalog 分片聚合 | Pending | wvp-GB28181-pro, simple-media-server | 已实现 | GB/T 28181-2022 附录 A | 当前为 synthetic fixture，真实设备需在 `GB4-SYS-003` 阶段补充 |
| PTZ/Preset/DragZoom | Pending | wvp-GB28181-pro, simple-media-server | 已实现 | GB/T 28181-2022 附录 C | 指令 XML 已验证，设备响应语义待真实验证 |
| 级联 Catalog/Alarm Notify | Pending | wvp-GB28181-pro, GB28181.Solution | 部分实现 | GB/T 28181-2022 第 9 章 | 上下级平台互操作在 `GB4-SYS-004` 阶段补充 |
| MD5 Digest compatibility | Pending | AKStream | 已实现 | GB/T 28181-2016 / profile | 仅在显式 profile 开启，待真实 2016 设备验证 |

## 3. 已完成的 Synthetic/Reference Peer 验证

- `testdata/gb28181/sip/`：7 组 SIP 报文与预期响应，全部通过 `cargo test` 与 `scripts/verify_gb4_fixtures.py`。
- `testdata/gb28181/xml/`：Keepalive、Catalog query/response 等 XML fixture，全部通过校验。
- `GB4-CAS-001..006`：平台级联模型 `GbPlatformLink`、`PlatformLinkRepository`、`CascadeManager`、loop/hop/ACL/唯一 control owner 已通过领域单测、存储 contract 和管理器单测，为真实上下级平台互操作提供控制面基线。
- `tools/gb28181-simulator` platform 模块：可本地模拟上下级注册、目录查询/通知、订阅和桥接 INVITE/200/ACK/BYE，用于预互操作冒烟，但不替代真实平台证据。
- 所有 fixture 来源标记为 `synthetic` 或 `reference-peer`，许可证 MIT-0，符合 clean-room 规则。

## 4. 已落地控制面任务补充

自本报告建立以来，以下 `GB4-*` 任务已完成并进入生产装配/测试，为真实互操作提供本地控制面基线：

| 任务 | 贡献 |
| --- | --- |
| `GB4-ACC-001..005` | 持久化 `ProtocolSession`、REGISTER/keepalive/deregister、DeviceInfo/DeviceStatus/Catalog/RecordInfo 聚合、目录事件落库 |
| `GB4-CMD-001..003` | typed device-control/PTZ/preset/query 命令、OperationStep outcome 分离、UnknownOutcome 处理 |
| `GB4-EVT-001..002` | 所有 GB 事件进入 application/outbox、优先级/合并/死信/队列满策略 |
| `GB4-MED-001..008` | live/playback/download/talk media Saga、`MediaBinding`/`MediaSession`/`Operation` 四模型、compensation/reconcile |
| `GB4-SIP-004..006` | Digest 凭据、多 listener/tenant 路由、endpoint/NAT 模型 |
| `GB4-CAS-001..006` | `GbPlatformLink` 平台模型、`CascadeManager`、目录/订阅/桥接状态机、loop/hop/ACL |
| `GB4-COMP-001..004` | compatibility profile schema/matching、vendor 覆盖、provenance fixtures |
| `GB4-SEC-001..004` | threat model、凭据/endpoint/网络区策略、日志脱敏 |
| `GB4-TST-001..004` | golden/property/fuzz corpus、状态机迁移表、driver-core-module contract、fixed-shard fault DSL |
| `GB4-SYS-001/002` | edge SQLite + fake media、cluster PostgreSQL/NATS 全 GB vertical system test |
| `GB4-SYS-005` | 安全、过载和敏感信息泄漏测试报告 |
| `GB4-SYS-006/007/008` | chaos/rolling upgrade、有界容量画像、24h/72h soak harness |
| `GB4-SYS-009` | x86_64/aarch64、SBOM/license、migration 和 release checklist |

这些本地验证结果与 `synthetic`/`reference-peer` 证据共同说明：在未接入真实对端前，控制面实现已具备可验证基线，但不可被误认为真实设备/平台互操作结论。

## 5. 待真实设备/平台联调项

- 至少两类真实设备或 NVR（`GB4-SYS-003`，报告模板已建立）。
- 至少一个上级/下级平台级联组合（`GB4-SYS-004`，报告模板已建立并关联 `GB4-CAS-001..006` 控制面模型）。
- 网络拓扑：公网/专网、NAT、IPv4/IPv6、UDP/TCP。
- 记录项：manufacturer/model/firmware、标准版本、profile、脱敏 semantic transcript、不支持能力。

## 6. 结论

当前 `GB4-REF-004` 处于 `Blocked`：已完成参考 peer、synthetic fixture、`GB4-CAS` 控制面基线和 simulator harness 的互操作映射，并补充了已落地 `GB4-*` 控制面任务的本地验证证据；`GB4-SYS-003` 与 `GB4-SYS-004` 报告模板已建立并附本地预验证映射，真实设备/平台证据需在获得外部对端后补充。

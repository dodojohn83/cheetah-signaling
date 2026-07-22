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

## 4. 待真实设备/平台联调项

- 至少两类真实设备或 NVR（`GB4-SYS-003`，报告模板已建立）。
- 至少一个上级/下级平台级联组合（`GB4-SYS-004`，报告模板已建立并关联 `GB4-CAS-001..006` 控制面模型）。
- 网络拓扑：公网/专网、NAT、IPv4/IPv6、UDP/TCP。
- 记录项：manufacturer/model/firmware、标准版本、profile、脱敏 semantic transcript、不支持能力。

## 5. 结论

当前 `GB4-REF-004` 处于 `Blocked`：已完成参考 peer、synthetic fixture、`GB4-CAS` 控制面基线和 simulator harness 的互操作映射；`GB4-SYS-003` 与 `GB4-SYS-004` 报告模板已建立，真实设备/平台证据需在获得外部对端后补充。

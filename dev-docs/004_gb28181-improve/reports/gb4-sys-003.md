# GB4-SYS-003：真实设备/NVR 互操作报告

- 任务：`GB4-SYS-003`
- 状态：`Blocked`
- 日期：2026-07-21

## 1. 目标

完成至少两类真实设备或 NVR 的互操作验证，覆盖 GB/T 28181-2022 与 2016、UDP/TCP、注册/续期/注销/保活、目录/状态/报警/位置、PTZ/预置位、直播/回放/下载/对讲、endpoint/NAT 与重复/迟到响应。

## 2. 测试矩阵

| 维度 | 设备 A | 设备 B |
| --- | --- | --- |
| 标准 | GB/T 28181-2022 | GB/T 28181-2016（MD5 profile） |
| 传输 | UDP | TCP |
| 类型 | IPC / NVR | IPC |
| 认证 | Digest SHA-256 | Digest MD5 |
| 网络 | 公网 NAT | 专网 |

## 3. 已准备的验收 checklist

- [ ] 注册/续期/注销/保活成功；
- [ ] Catalog 分片完整聚合；
- [ ] DeviceInfo/DeviceStatus 返回与 fixture 一致；
- [ ] RecordInfo 查询与回放 invite/bye；
- [ ] PTZ/Preset/HomePosition/DragZoom 指令与确认；
- [ ] Alarm/MobilePosition 上报；
- [ ] 直播 invite 200 OK + media callback 终止；
- [ ] 对讲 broadcast 与语音通道建立；
- [ ] 断网/重启/重复响应后的状态恢复。

## 4. 记录模板

每项验证记录：
- manufacturer / model / firmware；
- 网络拓扑（source/observed/Contact/Via received-rport）；
- 标准版本与 compatibility profile；
- 脱敏 semantic transcript（无 Authorization/密码/完整 body）；
- 不支持能力列表与对应 fallback 行为。

## 5. 当前已实现控制面基线

`GB4-ACC-001..005`、`GB4-CMD-001..003`、`GB4-EVT-001..002`、`GB4-MED-001..008` 与 `GB4-COMP-001..004` 已在仓库实现单设备/NVR 控制面闭环，并随 CI 运行：

- `crates/testing/cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs`：SQLite + fake media 单节点 edge 纵向测试，覆盖 REGISTER digest、keepalive、catalog、alarm、PTZ/command、live INVITE/200/ACK/BYE。
- `crates/testing/cheetah-gb-system-tests/tests/gb4_sys_002_cluster.rs`：PostgreSQL/NATS + fake media 集群纵向测试，覆盖跨节点 owner epoch、消息派分和 Operation 终态。
- `tools/gb28181-simulator`：fixed-shard deterministic simulator，可模拟设备注册、目录上报、告警、媒体协商和故障场景，用于预互操作冒烟与容量画像。
- `testdata/gb28181/profiles/`：厂商兼容性 profile（海康/大华/NVR 等 quirks）已建立，真实设备验证时可直接匹配。

这些基线验证控制面协议解析、状态迁移和领域行为，但不替代真实设备/NVR 的互操作证据。

## 6. 验收 checklist 与本地预验证映射

以下映射说明 checklist 中每项已在本地控制面通过哪些 `GB4-*` 任务、系统测试或 simulator 预验证；真实设备/NVR 证据仍需在获得外部对端后补充。

| Checklist | 本地预验证 | 证据位置 |
| --- | --- | --- |
| 注册/续期/注销/保活成功 | `GB4-ACC-001..005`、`GB4-SIP-004..006` 实现 REGISTER/keepalive/deregister 状态机与 Digest；`cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs`/`gb4_sys_002_cluster.rs` 覆盖 edge/cluster 完整生命周期 | `crates/testing/cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs`、`gb4_sys_002_cluster.rs`；`crates/protocols/cheetah-gb28181-module/src/access/` |
| Catalog 分片完整聚合 | `GB4-ACC-005` bounded catalog/record-info aggregation 与 `FragmentBuffer`；`GB4-EVT-001` 目录事件落库 | `apps/cheetah-signaling/src/gb_catalog_buffer.rs`；`crates/protocols/cheetah-gb28181-module/src/cascade/catalog.rs`、`src/xml/catalog.rs`；`crates/testing/cheetah-state-machine-tests/tests/catalog.rs` |
| DeviceInfo/DeviceStatus 返回与 fixture 一致 | `GB4-ACC-004` bootstrap query Operation；`GB4-CMD-001` typed device-control payloads | `crates/protocols/cheetah-gb28181-module/src/xml/device_info.rs`、`src/xml/device_status.rs`；`crates/testing/cheetah-state-machine-tests/tests/command.rs`、`cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs` |
| RecordInfo 查询与回放 invite/bye | `GB4-MED-001..008` 媒体 Saga、`GB4-CMD-001/003` 命令/查询 Operation、`GB4-WF-002/003` playback/talk workflow | `crates/protocols/cheetah-gb28181-module/src/xml/record_info.rs`、`src/media/tests/playback_tests.rs`；`crates/testing/cheetah-state-machine-tests/tests/media.rs`、`cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs` |
| PTZ/Preset/HomePosition/DragZoom 指令与确认 | `GB4-CMD-001` typed PTZ/preset/device-control；`GB4-MED-001..008` media command routing | `crates/protocols/cheetah-gb28181-module/src/xml/device_control.rs`；`crates/testing/cheetah-state-machine-tests/tests/command.rs`、`cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs` |
| Alarm/MobilePosition 上报 | `GB4-EVT-001` 所有 GB 事件进入 application handler 与 outbox；`GB4-EVT-002` 优先级/合并/死信 | `crates/protocols/cheetah-gb28181-module/src/xml/alarm.rs`、`src/xml/mobile_position.rs`；`crates/testing/cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs`、`tools/gb28181-simulator` |
| 直播 invite 200 OK + media callback 终止 | `GB4-MED-001..008`、`GB4-WF-001..004` live/playback/talk/stop workflow；`MediaPort` contract 与 fake media 纵向测试 | `crates/protocols/cheetah-gb28181-module/src/media/tests/invite_tests.rs`、`src/media/session.rs`；`crates/testing/cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs` |
| 对讲 broadcast 与语音通道建立 | `GB4-MED-001..008` 语音广播/对讲 command；`GB4-WF-003` playback/talk saga | `crates/protocols/cheetah-gb28181-module/src/media/tests/golden/talk_invite.sip`、`src/media/tests/`；`crates/testing/cheetah-state-machine-tests/tests/media.rs` |
| 断网/重启/重复响应后的状态恢复 | `GB4-SYS-006` chaos/rolling upgrade、`GB4-TST-004` deterministic fault DSL；`owner epoch`、`MediaBinding` fencing 与对账 | `crates/testing/cheetah-gb-system-tests/tests/gb4_sys_006_chaos.rs`、`gb4_sys_002_cluster.rs`；`tools/gb28181-simulator` |

## 7. 阻塞原因

当前环境未接入真实 GB28181 设备或 NVR；报告将在获得真实设备后补充。

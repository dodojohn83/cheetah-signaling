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

- `cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs`：SQLite + fake media 单节点 edge 纵向测试，覆盖 REGISTER digest、keepalive、catalog、alarm、PTZ/command、live INVITE/200/ACK/BYE。
- `cheetah-gb-system-tests/tests/gb4_sys_002_cluster.rs`：PostgreSQL/NATS + fake media 集群纵向测试，覆盖跨节点 owner epoch、消息派分和 Operation 终态。
- `tools/gb28181-simulator`：fixed-shard deterministic simulator，可模拟设备注册、目录上报、告警、媒体协商和故障场景，用于预互操作冒烟与容量画像。
- `testdata/gb28181/profiles/`：厂商兼容性 profile（海康/大华/NVR 等 quirks）已建立，真实设备验证时可直接匹配。

这些基线验证控制面协议解析、状态迁移和领域行为，但不替代真实设备/NVR 的互操作证据。

## 6. 阻塞原因

当前环境未接入真实 GB28181 设备或 NVR；报告将在获得真实设备后补充。

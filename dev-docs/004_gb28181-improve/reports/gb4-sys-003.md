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

## 5. 阻塞原因

当前环境未接入真实 GB28181 设备或 NVR；报告将在获得真实设备后补充。

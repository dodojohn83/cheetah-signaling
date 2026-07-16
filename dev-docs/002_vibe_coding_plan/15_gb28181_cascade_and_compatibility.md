# 15 GB28181 级联、路由与互通兼容

## 1. 范围

实现信令服务器作为下级平台向上级平台注册、保活、目录共享、订阅/通知和媒体点播，同时作为上级平台接入下级平台。南向资源与对上级暴露模型通过映射层隔离；首版支持树形级联，不支持任意环网。

## 2. 级联模型

`CascadePlatform` 包含上级 SIP 端点、本地平台 ID、realm、凭据引用、注册周期、keepalive、共享策略和兼容 profile。`CascadeResourceMap` 把内部 device/channel 映射为对上级稳定的国标 ID。

映射 ID 一经发布不得因节点迁移或数据库切换改变。冲突在配置/同步阶段失败，不能运行时覆盖。

## 3. 实现任务

### GB-CAS-001：上级注册客户端

- [x] 实现 REGISTER challenge/response、刷新、注销和退避。
- [x] 上级密码从 secret provider 获取，不进入普通配置快照。
- [x] 网络恢复后加入抖动，防止所有租户同时重注册。
- [x] 注册状态转换为统一平台连接事件。

> 完成：PR #20 `feat(phase-15): GB-CAS-001 upstream cascade REGISTER state machine`。验证：`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace`, `buf format/lint`, `cargo deny check` 全绿。

### GB-CAS-002：保活与健康

- [x] 按上级 profile 发送 Keepalive MESSAGE。
- [x] 区分 SIP 传输成功与业务 XML 响应成功。
- [x] 连续失败达到阈值才标记断开，恢复只产生一次事件。

> 完成：PR #21 `feat(phase-15): GB-CAS-002 keepalive MESSAGE and health`。验证同上。

### GB-CAS-003：目录共享

- [x] 支持白名单、标签、组织树和租户边界过滤。
- [x] 处理上级 Catalog 查询，分页生成响应并限制单包条目。
- [x] 资源变化采用增量通知；失败后可通过全量查询恢复。
- [x] 不向上级泄露内部 tenant ID、节点地址和协议凭据。

> 完成：PR #22 `feat(phase-15): GB-CAS-003 upstream catalog sharing with filtered pagination`。验证同上。

### GB-CAS-004：上级点播路由

- [x] 收到上级 INVITE 后解析目标映射和 SDP。
- [x] 创建桥接 Operation 和统一 MediaSession，调度媒体节点并创建 MediaBinding，再向下级设备发起独立 INVITE。
- [x] 上下级 dialog、CSeq、Call-ID 完全独立，由 bridge context 关联。
- [x] 任一侧 BYE/CANCEL/超时触发另一侧、MediaBinding 和 MediaSession 按 desired state 收敛清理。
- [x] 媒体节点承担必要的转发/转换，信令服务只下发控制。

> 完成：PR #23 `feat(phase-15): GB-CAS-004 upstream play bridge INVITE/ACK/BYE/CANCEL handling`。验证同上。

### GB-CAS-005：事件上报

- [x] 在线/离线、报警等按共享策略转换并发送 NOTIFY/MESSAGE。
- [x] 上报队列有界并可合并状态类事件。
- [x] 关键报警使用 outbox 和幂等键，状态快照允许覆盖旧事件。

> 完成：PR #24 `feat(phase-15): GB-CAS-005 upstream event reporting with bounded queue`。验证同上。

### GB-CAS-006：订阅与通知

- [x] 处理上级 Catalog/Alarm/MobilePosition 订阅、刷新和取消。
- [x] subscription 包含 requester、event package、expiry、filter 和 dialog/transaction context。
- [x] 到期由 timer wheel 清理；重复订阅按标准/profile 更新而非无限新增。
- [x] NOTIFY/MESSAGE 的序号、重试、去重和终止通知均有状态机测试。

> 完成：PR #25 `feat(phase-15): GB-CAS-006 upstream subscription manager`。验证同上。

### GB-CAS-007：下级平台接入

- [x] 下级平台作为 `PlatformLink` 聚合接入，注册认证与普通设备共享 SIP 核心但使用独立业务策略。
- [x] 主动查询/接收下级 Catalog，把目录项映射为带 source link 的内部 Device/Channel。
- [x] 处理下级增量上下线、报警和移动位置，并按租户策略向上游转发。
- [x] 上级点播路由到下级时维护两套独立 dialog 和 bridge context。
- [x] ID 映射、目录环、重复下级资源和 link 删除均有确定性清理规则。

> 完成：PR #26 `feat(phase-15): GB-CAS-007 lower-platform downstream access`。验证同上。

## 4. 环路与安全

- [ ] 配置层禁止同平台 ID 形成直接环路。
- [ ] 事件 extension 带内部 hop metadata 时限制最大 hop；对外不暴露实现细节。
- [ ] 上级只能访问显式共享资源，所有请求再次做 ACL 校验。
- [ ] 每上级平台独立限流、熔断和审计。

## 5. 兼容性工程

建立 `compat/gb28181/<vendor>/<profile>/`，每个 profile 包含脱敏报文、配置、期望行为和已知限制。新兼容分支必须先提交失败测试，再提交实现。

版本发布前至少完成：标准模拟器、GMV 参考行为、海康常见设备、大华常见设备和一个上级平台的互通矩阵；无法获得真实设备的项目明确标记为“模拟器验证”，不得写成已验证。

## 6. 验收标准

- [x] 上级重连不影响南向设备在线状态。
- [x] 上下级同时超时或重复 BYE 不泄漏 dialog/media session。
- [x] 10 万共享通道目录可分页响应且内存有界。
- [x] ACL、租户隔离和稳定 ID 有自动化测试。
- [x] 下级平台断开/重注册不破坏已持久化映射，上下游订阅到期无泄漏。

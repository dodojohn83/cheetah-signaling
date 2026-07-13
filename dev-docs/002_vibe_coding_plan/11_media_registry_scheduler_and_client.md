# 11 媒体节点注册、调度与控制客户端

## 1. 目标与边界

信令服务器只负责控制和媒体协商，不转发音视频负载。`cheetah-media-server-rs` 作为受管媒体节点，通过统一内部契约注册、续租、上报容量、接收会话命令并回调状态。

本章实现 `crates/media-control`，接口以 `05_proto_contracts_and_codegen.md` 为准；媒体侧需同步采用相同 Proto 版本。

## 2. 媒体节点模型

`MediaNode` 包含：

- `node_id`、`instance_id`、`zone`、`labels`
- `control_endpoint`、可公告的媒体地址集合
- 支持的传输、封装、编解码和会话类型
- `capacity`、`load`、`health`、`draining`
- `lease_until`、`generation`、`contract_version`

注册相同 `node_id` 但不同 `instance_id` 时视为进程替换，generation 增加，旧实例回调失效。

## 3. 实现任务

### MED-001：注册与健康

- [ ] 实现 gRPC `RegisterNode`、`RenewLease`、`ReportLoad`、`DeregisterNode`。
- [ ] 校验 mTLS 身份与声明 node ID 一致。
- [ ] lease 过期节点立即停止新调度，已有会话进入核查而非立即判死。
- [ ] draining 节点只允许停止/查询既有会话。

### MED-002：调度器

候选过滤顺序固定：协议能力 → 媒体能力 → 网络可达区域 → 租户约束 → 健康/非 draining → 容量。评分综合可用会话、带宽、CPU、区域亲和和稳定随机因子。

- [ ] 调度输入为不可变 `MediaRequirements`。
- [ ] 相同 session ID 重试优先选择原节点。
- [ ] 无候选时返回结构化原因集合，便于运维定位。
- [ ] 调度决策记录候选摘要和规则版本，不记录凭据。

### MED-003：控制客户端

- [ ] 连接池按 media node 管理，支持 TLS、keepalive、连接超时。
- [ ] 所有操作带 `request_id`、`session_id`、`deadline`、幂等键和 contract version。
- [ ] 仅对明确可重试错误重试；创建会话依靠幂等键避免重复资源。
- [ ] 实现断路器和每节点并发限制。

### MED-004：会话工作流

实时预览顺序：创建领域会话 → 调度节点 → 预留媒体资源 → 获取协议协商参数 → 协议 INVITE/启动 → 确认媒体节点 → Active。

任一步失败必须逆序补偿：终止协议事务、释放媒体资源、结束领域会话。补偿失败进入 reconciliation，不阻塞原错误返回。

- [ ] 实现 start/stop live。
- [ ] 实现 playback create/control/stop。
- [ ] 预留 talk 会话能力，首版可返回 capability unsupported。
- [ ] 回调处理校验 node generation、session revision 和消息 ID。

### MED-005：重启与对账

- [ ] 启动后分页查询媒体节点现存会话。
- [ ] 数据库有、媒体无：按策略重建或标记失败。
- [ ] 媒体有、数据库无：经过保护窗口后清理孤儿。
- [ ] 双方状态不同：以 session generation 和终态优先规则收敛。

### MED-006：媒体仓库旧 GB 模块迁移

- [ ] 从 `cheetah-media-server-rs` 旧 GB 模块提取脱敏 fixture、已知 quirk 和互通基线，记录许可证与来源。
- [ ] 新 gRPC/media API 与新信令链路先运行在隔离端口，禁止与旧 listener 同时拥有同一设备。
- [ ] 先 mirror 无副作用的目录/presence 事件，对比映射、错误和延迟；控制命令禁止双写。
- [ ] 按 tenant/device allowlist 灰度切换 SIP owner，比较在线率、点播成功率、首帧和残留 RTP session。
- [ ] 达到冻结观察窗口后关闭旧 listener；一个发布周期内保留显式回滚配置，回滚仍保证唯一 owner。
- [ ] 最终从媒体应用装配移除 GB 信令 module，只保留 RTP/RTCP、PS、demux/mux、转码等 Media Plane 能力。

## 4. 联调契约

媒体项目需要提供：

- 相同 Proto 的兼容性测试。
- mock media server 和故障注入开关。
- 创建/停止幂等保证及资源查询 API。
- 节点重启后的 session 枚举能力。
- 回调重试与签名/mTLS 行为说明。
- 旧 GB listener 的禁用、灰度和唯一 owner 开关。

## 5. 验收标准

- 信令进程抓包中不出现媒体 RTP/RTCP 负载。
- media node 故障不会阻塞其他节点调度。
- 重复创建、回调乱序、控制超时和进程重启均能通过对账收敛。
- 集成测试同时覆盖真实 gRPC server 与 mock 故障路径。

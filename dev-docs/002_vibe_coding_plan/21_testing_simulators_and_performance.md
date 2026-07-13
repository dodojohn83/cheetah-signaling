# 21 测试体系、模拟器与性能验证

## 1. 测试分层

测试按成本和信心分层：

1. 单元/属性测试：纯函数、状态机、解析器。
2. crate 集成测试：端口的内存或真实后端实现。
3. 协议模拟器测试：真实 socket/HTTP/SOAP。
4. 系统测试：数据库、NATS、媒体服务和多信令节点。
5. 性能/耐久/故障注入：独立环境、固定基线。

每个任务 PR 必须声明新增或更新的层级，禁止只以手工测试验收。

## 2. 测试基础设施

### TST-001：确定性测试

- [ ] 注入 Clock、ID generator、随机源和网络 fault policy。
- [ ] Tokio 时间测试使用 pause/advance。
- [ ] 测试 seed 在失败输出，允许本地复现。
- [ ] 禁止依赖测试执行顺序、公共端口或公网。

### TST-002：协议模拟器

建立：

- `tools/gb28181-simulator`：多设备注册、心跳、目录、报警、INVITE/回放及畸形行为。
- `tools/onvif-simulator`：Discovery、Device/Media/PTZ/Events、WSSE、时钟漂移和 Fault。
- `tools/media-control-simulator`：注册、容量、会话、回调、延迟与失败注入。

模拟器配置可用 seed 批量生成稳定设备 ID；支持速率曲线、断线重连和厂商 profile。

### TST-003：Golden 与抓包

- [ ] 脱敏真实 SIP/XML/SDP/SOAP 样本进入 testdata。
- [ ] 每样本包含来源类别、标准/profile、期望结果和脱敏说明。
- [ ] 关键互通场景保存 pcap 或语义 transcript，CI 比较规范化输出。
- [ ] 禁止提交真实密码、token、证书私钥和公网地址。

### TST-004：Fuzz/属性测试

- [ ] SIP、SDP、GB XML、SOAP/XML、Proto decode、cursor decode 均建 fuzz target。
- [ ] CI 跑短时 fuzz，夜间跑长时 corpus；崩溃样本回归入库。
- [ ] 解析器属性：不 panic、受限分配、往返语义、拒绝歧义长度。

## 3. 系统场景

- [ ] 单机 SQLite：GB/ONVIF 纳管、Operation/Command 派发、开停流、重启恢复。
- [ ] 三节点 PostgreSQL+NATS：设备分布、Operation 恢复、Command 路由、owner 接管。
- [ ] 媒体节点扩缩、drain、故障、回调乱序、MediaBinding 迁移与孤儿清理。
- [ ] 验证 Operation、MediaSession、MediaBinding 终态独立：Start Operation 成功不终止 Active Session，Stop Operation 成功后无有效 Binding。
- [ ] 滚动升级 N → N+1，持续业务不破坏契约。
- [ ] PostgreSQL/NATS 短暂中断与恢复。
- [ ] 多租户相同外部设备 ID 严格隔离。

## 4. 性能计划

性能报告必须记录 commit、Rust/toolchain、硬件、内核、数据库/NATS 版本、配置、数据集、预热和持续时间。

### PERF-001：边缘基线

目标硬件至少选择一个 ARM64 四核/8GB 或更低设备：

- 空载 RSS、启动时间、二进制/镜像大小。
- 1k/10k 设备心跳、SQLite 写入节流、事件查询。
- 100/1000 并发会话控制（媒体使用模拟器）。
- 开发阶段运行 24 小时耐久；发布候选运行 72 小时，检查内存增长、文件句柄和数据库大小。

### PERF-002：百万设备集群

测试规模逐级为 10 万、30 万、100 万设备。设备并非全部同时活跃；明确在线比例、心跳周期、目录大小、Operation/Command 速率和重连曲线。

测量：注册 TPS、心跳 TPS、Operation 吞吐与终态延迟、Command 派发吞吐、端到端 P50/P95/P99、owner 分布、DB/NATS 负载、每节点 RSS/CPU、故障恢复时间和丢弃/重复率。

- [ ] 在线比例、GB/ONVIF 分布、心跳/presence TTL、报文大小、TLS 比例和 1%/5%/10% 媒体活跃比例写入场景清单。
- [ ] 分别埋点 API 接收、调度、设备协议、媒体准备和媒体首帧延迟。
- [ ] 运行 72 小时稳态测试，证明队列、timer、连接、内存、outbox/inbox 无无界增长。
- [ ] 输出 socket/内核、PostgreSQL、NATS 和文件句柄推荐参数及推导依据。

### PERF-003：风暴与背压

- [ ] 10%/50% 设备在抖动窗口内重连。
- [ ] NATS/DB 降速时验证有界队列和明确拒绝。
- [ ] 大目录、报警洪水、慢 ONVIF 设备不会拖垮其他租户。
- [ ] 过载恢复后 backlog 在目标时间内排空且无无限重试。

## 5. 发布门禁

- 所有单元、集成、契约、系统测试通过。
- fuzz corpus 无未分类 crash。
- 性能相对已发布基线退化超过冻结阈值时必须阻止发布或获得书面例外。
- 竞态敏感模块使用 loom/shuttle 等模型测试或提供等价证明性测试。
- 关键 unsafe 代码有 Miri/平台测试和人工审查。

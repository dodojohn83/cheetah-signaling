# 01. 目标、容量与 SLO

## 1. 产品目标

建设一个可在 ARM 边缘盒子轻量部署、也可水平扩展至百万在线设备的统一信令控制面。协议增加不应迫使核心领域模型、媒体服务或北向 API 重写。

首版必须完成：

- GB28181 设备接入与平台级联的生产闭环；
- ONVIF 设备发现、能力获取、媒体拉取、PTZ、事件和抓图闭环；
- 与 `cheetah-media-server-rs` 的稳定控制契约；
- edge 与 cluster 两种可验证部署；
- 多租户、鉴权、审计、可观测和有界故障恢复；
- 未来协议使用的插件 SDK 与能力协商协议。

## 2. 容量口径

“在线”是协议相关的业务状态，不强制等同于持久 TCP 连接：

- GB28181：注册未过期且保活未超时；TCP/UDP transport 独立统计。
- ONVIF：最近一次探测或控制请求成功且未超过配置的 presence TTL；PullPoint subscription 另行统计。
- 级联平台：注册有效且平台健康检查正常。

集群目标：

- 100 万同时在线设备；
- 通道数量允许高于设备数量，所有目录查询必须分页；
- 媒体控制会话与在线设备数分开建模，测试按 1%、5%、10% 活跃比例压测；
- 单一 tenant、单一厂商或单一节点不得形成全局锁或全局队列；
- 增加 gateway 节点后，连接、保活和命令吞吐应近似水平扩展。

容量声明必须绑定硬件、内核参数、协议分布、保活周期、报文大小、TLS 比例和活跃操作比例。文档不得宣称某固定单节点设备数，除非有可复现报告。

## 3. SLO

cluster 生产配置的设计目标：

| 指标 | 目标 |
| --- | --- |
| 北向控制 API 可用性 | 月度 99.95%，不含外部设备和媒体节点故障 |
| 已接受配置/Operation RPO | 0；要求 PostgreSQL 同步 HA 和成功提交 outbox |
| gateway 故障检测 | 15 秒内 |
| 新 owner 可接管 | 30 秒内；依赖设备重注册/重连的流程除外 |
| 内部事件 | 至少一次，可重放、可去重 |
| 命令副作用 | 通过幂等键、inbox 和 fencing 实现最多一次有效副作用 |
| 内存与队列 | 72 小时稳态无无界增长 |

设备响应延迟和媒体首帧延迟必须拆分：API 接收、调度、设备协议、媒体准备、媒体首帧分别埋点，不能用一个总延迟掩盖瓶颈。

edge 配置不承诺进程级 HA，但必须在异常退出后通过 SQLite 恢复配置、未完成 Operation 和必要的补偿工作。

## 4. 首版功能范围

### 4.1 GB28181

- REGISTER/Digest、注销、保活、离线判定；
- Catalog、DeviceInfo、DeviceStatus；
- 实时点播、停止、回放、下载；
- PTZ、告警、移动位置、语音对讲；
- 向上级平台注册、目录共享、订阅/通知、点播转发；
- 下级平台接入、通道映射、级联事件转发；
- UDP/TCP SIP，媒体 RTP transport 由媒体节点能力决定；
- GB/T 28181-2022 主路径与 2016 兼容策略。

### 4.2 ONVIF

- WS-Discovery 分区发现；
- Device Management、Capabilities、时间同步信息；
- Media2 主路径与 Media1 fallback；
- Profile、encoder configuration、stream URI、snapshot URI；
- PTZ、Imaging 基础读取；
- Event PullPoint 创建、续期、拉取和取消；
- Profile T 主路径，Profile S legacy 兼容。

Profile G 完整录像管理、Profile M 分析元数据、ONVIF Device 模拟不属于 v1。

## 5. 非目标

- 不解析、转发或存储 RTP/RTCP/PS/ES 媒体包；
- 不在信令服务内实现 RTSP 拉流客户端、录制、转码或播放输出；
- 不保证 gateway 故障时活 TCP 连接或 SIP dialog 无感迁移；
- 不用分布式事务覆盖设备、数据库、消息系统和媒体节点；
- 不用一个万能 JSON 模型表达所有协议；
- 不为未来协议提前实现未验证的业务语义；
- 不承诺 Profile S 的安全性，只提供显式开启的兼容能力。

## 6. 维护目标

- 架构、wire schema、数据库和扩展点应能演进十年以上；不承诺某个二进制十年不升级。
- 公共 REST v1 和 Protobuf v1 只允许兼容扩展；破坏性升级必须新建 major 并提供双栈窗口。
- 每年发布一个 LTS 基线，安全修复和协议兼容补丁与功能版本分离。
- Rust stable 和依赖至少每半年评估升级；升级工具链不能顺带改变公开协议行为。

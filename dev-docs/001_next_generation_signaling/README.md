# Cheetah 下一代信令服务器设计

## 1. 文档定位

本目录定义独立信令控制面的目标架构和首个生产版本。它是实现、评审和验收的规范性输入，不是概念草图。

系统使用 Rust 编写，首版实现 GB/T 28181 与 ONVIF，后续通过相同领域端口和进程插件协议接入 HomeKit、Matter、MQTT、SIP、RTSP Control、海康 ISUP、大华 SDK、JT808 等协议。

信令服务只处理设备身份、协议状态机、控制命令、工作流与集群管理。RTP、RTSP 拉流、转封装、录制和播放输出全部由 `cheetah-media-server-rs` 承担。任何媒体包都不得穿过信令进程。

## 2. 已冻结的设计结论

1. 单套代码支持两种部署配置：
   - edge：单进程、SQLite、本地总线，可与本机媒体服务通过 UDS 或 loopback gRPC 通信。
   - cluster：PostgreSQL、NATS Core、JetStream、NATS KV，多角色水平扩展。
2. 容量目标为单集群 100 万同时在线设备，不要求单节点承载 100 万连接。
3. 内置协议统一采用 `core + driver-tokio + module`；`core` 是 Sans-I/O 状态机。
4. 百万设备运行时使用固定数量的分片 worker，不为每个设备创建独立 Tokio task。
5. GB28181 首版同时支持设备接入和上下级平台级联，以 2022 标准为主并兼容 2016 设备。
6. ONVIF 首版是 client/controller，优先 Profile T + Media2，兼容 Media1/Profile S。
7. 插件稳定边界是进程间 Protobuf/gRPC；不承诺 Rust 动态库 ABI。
8. 公共业务接口为 REST/OpenAPI + SSE/Webhook；gRPC 只用于内部、插件和媒体节点。
9. 所有资源原生支持多租户。单机模式使用默认租户，但不省略 `TenantId`。
10. HA 采用租约、所有权 epoch、fencing、幂等和补偿工作流；不迁移活 TCP 连接或 SIP dialog。

## 3. 规范优先级

发生冲突时依次遵循：

1. 国家/行业协议标准及对应勘误；
2. 本目录中明确写为“必须”的内部契约；
3. `cheetah-media-server-rs/dev-docs/901_api_plan` 的媒体领域边界；
4. 真实设备兼容 fixture 和厂商差异策略；
5. 具体库或框架的默认行为。

不得为了复用现有实现而破坏控制面与媒体面的边界。媒体仓库现有 GB28181 module 只能作为兼容样例和迁移来源，不能成为新信令服务的运行时依赖。

## 4. 文档索引

| 文档 | 内容 |
| --- | --- |
| [01_goals_and_slo.md](01_goals_and_slo.md) | 目标、容量、SLO、范围和非目标 |
| [02_architecture_and_deployment.md](02_architecture_and_deployment.md) | 分层、组件、并发模型、edge/cluster 部署 |
| [03_domain_and_storage.md](03_domain_and_storage.md) | 统一领域模型、状态所有权、数据库和凭据 |
| [04_internal_contracts_and_api.md](04_internal_contracts_and_api.md) | Protobuf、NATS、插件协议、REST 和事件接口 |
| [05_media_plane_integration.md](05_media_plane_integration.md) | 媒体节点注册、调度、MediaKey 和工作流 |
| [06_gb28181_design.md](06_gb28181_design.md) | GB28181 设备接入、级联、兼容和状态机 |
| [07_onvif_design.md](07_onvif_design.md) | ONVIF 发现、服务、Profile 和兼容策略 |
| [08_ha_security_observability.md](08_ha_security_observability.md) | HA、安全、可观测、升级和维护策略 |
| [09_testing_and_acceptance.md](09_testing_and_acceptance.md) | 单测、互操作、故障、容量和验收标准 |
| [10_implementation_roadmap.md](10_implementation_roadmap.md) | crate 规划、阶段顺序、迁移与完成定义 |
| [11_reference_baseline.md](11_reference_baseline.md) | 标准、技术资料、参考实现与版本基线 |

## 5. 术语

- **设备（Device）**：统一资产，不等同于某个协议中的外部编号。
- **端点（Endpoint）**：设备可访问的协议地址，例如 SIP Contact 或 ONVIF XAddr。
- **通道（Channel）**：可独立控制或产生媒体的逻辑通道。
- **协议会话（ProtocolSession）**：REGISTER、SIP dialog、ONVIF subscription 等临时控制状态。
- **媒体会话（MediaSession）**：用户视角的逻辑媒体意图和 desired state，不等同于媒体节点资源。
- **媒体绑定（MediaBinding）**：MediaSession 与具体媒体节点 session/MediaKey/handle 的物理关联。
- **Operation**：对外可查询、可取消或可超时的异步控制操作。
- **Command**：由 Operation/Saga 派发的不可变指令；其投递状态不构成第二套业务生命周期。
- **owner epoch**：设备会话所有权每次变更时递增的 fencing token。
- **协议 module**：内置信令能力。本文不用“动态插件”指代内置 module。
- **进程插件**：独立进程，通过版本化协议接入未来协议或厂商 SDK。

## 6. 设计基线

- Rust Edition 2024；初始工具链固定到项目启动时验证通过的 stable 版本。
- Tokio 作为首个运行时；公共领域 trait 不暴露 Tokio channel、task 或 timer 类型。
- Protobuf 是跨进程和持久消息的规范 wire schema；Rust struct 不是跨进程 ABI。
- PostgreSQL 是 cluster 权威存储，SQLite 是 edge 权威存储。
- JetStream 采用至少一次语义，业务通过 inbox/outbox 和幂等键实现 exactly-once effect。
- 所有队列、缓存、解析深度、重试、批次、分页和订阅窗口必须有上界。

标准与组件版本不得凭记忆升级；实现每个 phase 前应复核 [参考基线](11_reference_baseline.md)，记录实际采用版本和差异。

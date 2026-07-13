# 02. 架构与部署

## 1. 总体结构

```text
Devices / Platforms
        │ GB28181 / ONVIF / future protocols
        ▼
Protocol Gateways ──► sharded protocol state machines
        │                         │
        │                         ▼
        │                Application / Workflow
        │                 │       │       │
        │                 ▼       ▼       ▼
        │              Storage   Bus   Ownership
        │                         │
        ▼                         ▼
 Media Scheduler ───────► Media Node Control
                              │
                              ▼
                   cheetah-media-server-rs

Northbound REST/SSE/Webhook ─► Application / Workflow
External protocol plugins ───► PluginRuntime gRPC
```

控制面内部通过 ports 隔离部署差异。domain 和 protocol-core 不知道当前使用 SQLite 还是 PostgreSQL、local bus 还是 NATS、UDS 还是 TCP gRPC。

## 2. 分层与依赖方向

从上到下分为：

1. **apps/assembly**：配置加载、进程角色、依赖注入、信号处理。
2. **transport adapters**：REST、gRPC、NATS、数据库、secret provider。
3. **application**：命令处理、Operation、Saga、reconciler、权限和配额。
4. **protocol module**：协议业务映射、设备/通道同步、媒体编排请求。
5. **protocol driver**：socket、HTTP/TLS、framing、连接、timer 驱动。
6. **protocol core/foundation**：Sans-I/O 状态机、codec、领域类型和 ports。

依赖只能向下或指向抽象 port。禁止：

- protocol-core 依赖 Tokio、Axum、SQLx、NATS、tonic 或媒体 client；
- application 直接操作具体 SQL connection；
- 协议 module 绕过 `MediaPort` 连接媒体 engine；
- HTTP DTO、SOAP DTO 或 SIP message 直接作为领域实体持久化；
- 插件进程获得数据库、NATS 或媒体节点的通用凭据。

## 3. 协议三段式

每个内置协议固定为：

```text
cheetah-<protocol>-core
cheetah-<protocol>-driver-tokio
cheetah-<protocol>-module
```

`core` 接口使用显式 `Input / Output / Event / TimerId / Command`。时间由 driver 注入，输出动作由调用方执行。解析器必须支持增量输入、大小限制和明确错误位置。

`driver` 将 socket/HTTP 事件转换成 core input，并执行 write、timer 和 close output。每条连接有有界读写缓冲；慢连接只能阻塞或关闭自身。

`module` 将协议事件映射成统一领域命令和事件，维护业务映射，但不重复实现 wire state machine。

## 4. 百万设备并发模型

### 4.1 分片 worker

- `DeviceId` 或尚未认证时的 protocol routing key 经稳定 hash 映射到本节点 worker。
- worker 数量启动时固定，默认按 CPU 核数和实测调整，不随设备数线性增长。
- 每个 worker 独占其 session map、transaction map 和 timer wheel，热路径不使用跨 worker mutex。
- listener/HTTP executor 通过有界队列投递输入；队列满时按协议执行限流、503/Retry-After、丢弃低价值 telemetry 或断开连接。
- 跨 worker 操作使用消息，不共享可变 session。

### 4.2 定时器

保活、REGISTER 到期、SIP transaction、ONVIF renewal 和 Operation deadline 使用分层时间轮或等价的批量 timer。禁止每个设备创建独立 Tokio sleep task。

### 4.3 状态分类

- 热状态：连接、transaction、dialog、subscription cursor，保存在 owner 节点内存。
- 温状态：presence、owner directory、节点租约，保存在 NATS KV/内存投影。
- 冷状态：资产、配置、Operation、outbox、审计，保存在 PostgreSQL/SQLite。

## 5. edge 部署

edge profile 默认包含：

- REST API；
- GB28181 与 ONVIF module/driver；
- application/workflow；
- SQLite WAL repository；
- in-process event bus、ownership directory 和 scheduler；
- media gRPC client；媒体服务同机时优先 UDS。

edge 可编译成单一可执行文件。NATS、PostgreSQL、cluster coordinator 和外部 plugin host 均为可关闭 feature/role，不影响领域接口。

推荐支持 `aarch64-unknown-linux-gnu` 与 `aarch64-unknown-linux-musl`；ARMv7 作为兼容目标。不能假设边缘设备有 systemd、容器运行时或公网 DNS。

## 6. cluster 部署

同一个二进制通过 role 组合运行：

- `api`：REST/SSE、鉴权、查询和 Operation 创建；
- `gateway-gb`：SIP UDP/TCP 接入；
- `worker-onvif`：ONVIF HTTP、PullPoint、轮询任务；
- `discovery-agent`：指定 network zone 的 WS-Discovery；
- `workflow`：Saga、outbox publisher、reconciler；
- `plugin-host`：管理外部协议进程；
- `all`：小型集群或 edge 的组合角色。

生产 cluster 至少包含：

- PostgreSQL HA；
- 3 节点 NATS/JetStream；
- 2 个以上 API 节点；
- 每种启用 gateway/worker role 至少 2 个实例；
- L4 负载均衡，分别处理 SIP UDP、SIP TCP 和 HTTPS。

gateway 连接由接入节点持有。负载均衡变更或节点故障后，设备通过重注册/重连建立新 owner，不转移既有 socket。

## 7. 背压与资源治理

每个 tenant、协议、节点和设备至少配置：

- 最大连接数、请求速率和并发 Operation；
- 最大 XML/SIP body、header 数、目录条目和分页大小；
- 最大在途 transaction、dialog、event subscription；
- 最大 webhook 重试、NATS batch、outbox batch；
- 最大媒体 session、RTP port reservation 和 pending invite；
- 空闲、握手、设备响应和总 Operation deadline。

任何默认值都必须出现在配置 schema 和运维文档中。不能用“无限”作为生产默认值。

## 8. 进程生命周期

启动顺序：配置/secret → storage migration check → NATS/本地 bus → repositories → ownership → media registry/client → protocol modules → public listeners → ready。

关闭顺序：撤销 ready → 停止接收新 Operation/连接 → 停止获取新租约 → 有界 drain → 持久化 outbox/Operation → 关闭协议连接 → 释放媒体 binding → 关闭存储。

ready 只表示当前角色能完成其职责；例如 gateway 在 NATS 或 ownership 不可用时不能继续宣告 ready 并接受可能形成 split-brain 的新注册。

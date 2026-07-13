# Cheetah Signaling 系统架构

## 部署角色

- **edge**：单进程、SQLite、本地消息总线，可本地媒体服务通过 UDS/loopback gRPC 通信。
- **cluster**：PostgreSQL、NATS Core / JetStream / KV，多角色水平扩展。

## 六层依赖

1. `apps/cheetah-signaling`：配置、装配、生命周期。
2. transport adapters：HTTP、gRPC、NATS、数据库、secret provider。
3. application：用例、Operation、Saga、reconciler、权限、配额。
4. protocol module：协议业务到统一领域模型的映射。
5. protocol driver：socket、HTTP/TLS、framing、连接、timer 驱动。
6. protocol core / foundation：Sans-I/O 状态机、codec、领域类型、ports。

依赖只能向下或指向本层定义的抽象 port。

## 协议三段式

每个内置协议拆分为：

- `cheetah-<protocol>-core`：Sans-I/O 状态机。
- `cheetah-<protocol>-driver-tokio`：网络/时钟事件转换。
- `cheetah-<protocol>-module`：领域映射与业务编排。

## 媒体边界

信令进程只处理控制面：设备身份、协议状态机、命令、媒体协商与资源生命周期。
禁止接收、转发、解析或存储 RTP/RTCP/PS/TS/ES 媒体负载，禁止绑定媒体 RTP/RTCP 端口。

## 数据流

设备 → 协议 driver/core → protocol module → application → domain → storage/outbox → bus → 媒体/HTTP/SSE/Webhook。

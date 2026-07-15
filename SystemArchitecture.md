# System Architecture

本文件记录 `cheetah-signaling` 已冻结的分层、部署角色和数据流，只描述当前已确定的设计，不引入新决策。

## 1. 分层（从上到下）

1. `apps/assembly`：配置加载、角色装配、依赖注入、进程生命周期。
2. `transport adapters`：HTTP/gRPC/NATS/数据库/secret provider。
3. `application`：命令处理、Operation、Saga、reconciler、权限与配额。
4. `protocol module`：协议业务映射、设备/通道同步、媒体编排请求。
5. `protocol driver`：socket/HTTP/TLS/framing/连接/timer 驱动。
6. `protocol core/foundation`：Sans-I/O 状态机、codec、领域类型和 ports。

依赖只能向下或指向同层定义的抽象 port。domain 和 core 不依赖 Tokio、Axum、Tonic、SQLx、async-nats、quick-xml 或媒体 client。

## 2. 部署角色

- **edge**：单进程、SQLite、本地总线、本地媒体 gRPC/UDS。
- **cluster**：PostgreSQL、NATS Core/JetStream/KV、多角色水平扩展。

角色通过 `apps/cheetah-signaling` 启动参数装配，不影响领域接口。

## 3. 协议三段式

每个内置协议拆分为：

```text
cheetah-<protocol>-core
cheetah-<protocol>-driver-tokio
cheetah-<protocol>-module
```

core 使用显式 `Input/Output/Event/TimerId/Command`，driver 将网络/时钟事件转换为 core input，module 完成业务映射。

## 4. 并发模型

- 固定数量的分片 worker，不为每个设备创建独立 Tokio task。
- worker 持有 session map、transaction map 和 timer wheel，热路径不跨 worker 加锁。
- listener 通过有界队列投递输入；队列满时执行限流/断开/503。
- 跨 worker 操作通过消息，不共享可变 session。

## 5. 状态分类

- 热状态：连接、transaction、dialog、subscription cursor（owner 节点内存）。
- 温状态：presence、owner directory、节点租约（NATS KV/内存投影）。
- 冷状态：资产、配置、Operation、outbox、审计（PostgreSQL/SQLite）。

## 6. 媒体面边界

信令进程不接收、转发、解析或存储 RTP/RTCP/PS/TS/ES 媒体负载；不绑定媒体端口；不解码/转码/录制/生成播放 URL。媒体请求通过 `cheetah.media.v1` 契约和 `MediaPort`/媒体客户端执行。

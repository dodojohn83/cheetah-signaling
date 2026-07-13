# 04. 内部契约、插件与北向 API

## 1. Protobuf 包与版本

规范包固定为：

```text
cheetah.common.v1
cheetah.device.v1
cheetah.control.v1
cheetah.plugin.v1
cheetah.media.v1
cheetah.cluster.v1
```

package major 是 wire major。v1 内只允许兼容扩展：不能修改字段号/类型，不能复用删除字段号，enum 必须保留 unspecified/unknown，接收端必须容忍未知字段和未知 enum 数值。

CI 执行 format、lint、生成代码一致性和 `buf breaking --against`。生成的 Rust crate 采用 SemVer 发布，媒体仓库和外部插件只依赖已发布版本，不能使用跨仓库相对路径。

## 2. Envelope

规范字段：

```proto
message EnvelopeMeta {
  string message_id = 1;          // UUIDv7
  string tenant_id = 2;
  string correlation_id = 3;
  optional string causation_id = 4;
  google.protobuf.Timestamp occurred_at = 5;
  optional google.protobuf.Timestamp deadline = 6;
  string source_node_id = 7;
  uint64 owner_epoch = 8;
  optional string traceparent = 9;
  optional string tracestate = 10;
}

message ResourceRef {
  string resource_type = 1;
  string resource_id = 2;
}

message CommandEnvelope {
  EnvelopeMeta meta = 1;
  ResourceRef target = 2;
  string idempotency_key = 3;
  oneof command { /* typed commands */ }
}

message EventEnvelope {
  EnvelopeMeta meta = 1;
  ResourceRef aggregate = 2;
  uint64 aggregate_sequence = 3;
  oneof event { /* typed events */ }
}
```

厂商扩展可使用 `google.protobuf.Any`，但 type URL 必须在 capability handshake 注册，payload 有大小上限，核心不能依赖未知扩展完成安全决策。

## 3. 传输选择

同一 envelope 用于不同可靠性需求：

- 进程内：有界 channel，edge 主路径；
- gRPC/UDS：进程插件、媒体节点、必要的节点直连；
- NATS Core：健康探测、短生命周期 request/reply；
- JetStream：需要持久、重放、重试的命令和领域事件；
- NATS KV：节点租约、ownership directory 和 capability 快照。

协议回复（例如 SIP 200 OK）不经过 JetStream；它属于 gateway 本地 transaction 热路径。业务控制 Operation、媒体编排和跨节点命令必须使用可追踪 envelope。

## 4. 进程插件协议

信令 host 暴露：

```text
PluginRuntime.Connect(stream PluginFrame) returns (stream HostFrame)
```

插件主动连接 host 的 UDS 或 mTLS endpoint。首帧必须是 `PluginHello`：

- plugin ID、实现版本、协议类型；
- 支持的 API major/minor；
- command/event type URLs；
- listener/egress 能力；
- 最大 inflight、建议 heartbeat；
- 配置 schema digest。

host 返回 `HostWelcome`：选定版本、instance ID、tenant/zone scope、流控窗口、heartbeat 和配置 revision。无法协商 major 时拒绝连接，不进行隐式降级。

运行帧包括 ConfigSnapshot/Delta、Command、CommandResult、DomainEvent、Health、Credit、Drain、Shutdown。双方都必须实现：

- seq/ack 和有限重放窗口；
- deadline 与 cancellation；
- 最大消息大小和 inflight；
- 幂等 command result；
- 健康超时和有界重启退避。

插件只能通过 host 请求设备/媒体/secret 的受限 capability。不能获得数据库 DSN、NATS 管理权限或任意 secret 枚举能力。

## 5. 集群内部服务

### 5.1 NodeCommand

`Execute(CommandEnvelope) -> CommandAccepted/CommandResult`。请求必须带目标 device owner epoch；节点先校验 session 与 epoch，再接受副作用。owner 不匹配返回 `StaleOwner` 和可选当前 owner hint。

### 5.2 Registry

`RegisterNode`、`HeartbeatNode`、`DrainNode` 用于 signaling/media/plugin 节点。descriptor 包含 role、zone、endpoint、build、protocol/media capabilities、capacity 和 labels。租约到期后节点立即不可调度。

### 5.3 Capability negotiation

任何调用方必须先读取 capability/version，不能通过调用并解析 “not implemented” 猜测支持情况。错误需区分 `Unsupported`、`Unavailable`、`Busy` 和 `VersionMismatch`。

## 6. REST API

路径前缀：`/api/v1/tenants/{tenant_id}`。tenant 必须同时与 token scope 匹配，不能仅信任 path。

资源组：

```text
/devices
/devices/{device_id}/endpoints
/devices/{device_id}/channels
/channels/{channel_id}
/platform-links
/operations
/media-sessions
/event-subscriptions
/webhooks
/protocol-capabilities
```

控制操作使用明确资源或 action endpoint，不提供万能的未校验 JSON command。例如创建 live session 返回 Operation 与 media session reference；PTZ command 使用固定 enum 和具名 speed/timeout 字段。

规则：

- mutating request 支持 `Idempotency-Key`；
- 长操作返回 `202 Accepted` 和 Operation URL；
- 更新资源使用 revision/ETag 防止 lost update；
- 列表使用 opaque cursor、稳定排序和最大 page size；
- 时间为 RFC3339 UTC，duration 明确使用毫秒字段；
- 错误为 RFC 9457 `application/problem+json`，扩展包含 stable code、request ID、retryable 和 field violations；
- OpenAPI 3.1 文件进入兼容性检查，不能从运行时 handler 猜测真实契约。

## 7. SSE 与 Webhook

SSE endpoint 支持 tenant、event kind、device/channel filter 和 `Last-Event-ID`。服务端设置最大订阅数、发送缓冲和 replay 范围；落后消费者收到显式 gap 事件后重新查询状态。

Webhook 投递：

- 使用 event ID 作为幂等标识；
- HMAC 签名覆盖 timestamp、delivery ID 和 raw body；
- 有 connect/request deadline、指数退避、最大次数、熔断和 dead-letter 状态；
- 默认拒绝 loopback、link-local、metadata service 和未允许的内网目标；
- DNS 每次重试重新校验，防止 DNS rebinding；
- Webhook 失败不能阻塞协议或媒体热路径。

## 8. 错误模型

稳定领域错误至少包括：

```text
InvalidArgument, Unauthenticated, PermissionDenied,
NotFound, AlreadyExists, Conflict, StaleOwner,
Busy, RateLimited, Timeout, Cancelled,
Unavailable, Unsupported, VersionMismatch,
ProtocolFailed, MediaFailed, StorageFailed, Internal
```

adapter 映射 HTTP/gRPC/NATS 表示；领域错误不携带 HTTP status。外部响应不得暴露 SQL、节点地址、栈回溯、secret 或原始设备报文。

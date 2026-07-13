# 04. 基础类型、错误与配置

## 1. 公共类型

实现受校验 newtype：`TenantId/DeviceId/EndpointId/ChannelId/ProtocolSessionId/MediaSessionId/OperationId/NodeId/PluginId/EventId/MessageId/CorrelationId`。内部 ID 为 UUIDv7；协议外部 ID 使用 `ProtocolIdentity`。

实现：`UtcTimestamp`、`DurationMs`、`Deadline`、`Revision(u64)`、`OwnerEpoch(u64)`、`PageRequest/Page<T>`、`RequestContext`、`Principal`、`ResourceRef`。

## 2. 错误

`SignalErrorKind` 固定包含 InvalidArgument、Unauthenticated、PermissionDenied、NotFound、AlreadyExists、Conflict、StaleOwner、Busy、RateLimited、Timeout、Cancelled、Unavailable、Unsupported、VersionMismatch、ProtocolFailed、MediaFailed、StorageFailed、Internal。

`SignalError` 包含 kind、稳定 code、安全 message、retryable、field violations、correlation ID；source 仅内部保存。禁止以字符串判断错误类型。

## 3. Ports

- `Clock`：wall time + monotonic deadline，测试可推进。
- `IdGenerator`：UUIDv7/Event/Message ID。
- `SecretStore`：put/get/delete/rotate by reference，无枚举明文接口。
- `RuntimeApi`：spawn Send future、cancellation、bounded sleep/timeout；公共接口不返回 Tokio handle。
- `ConfigSource`：snapshot/watch revision，支持 static/module-restart/dynamic/secret change effect。

## 4. 配置模型

根 `SignalConfig` 分 system、runtime、http、grpc、storage、messaging、cluster、media、plugins、gb28181、onvif、security、observability。每字段有 serde default、validator、敏感标志和 effect。

优先级固定：内置默认 < 配置文件 < 环境变量 < secret provider；运行期动态配置来自权威配置源，不能被环境变量热覆盖。未知字段默认拒绝。

## 5. 任务与测试

- [ ] 实现所有 newtype 的 parse/display/serde/prost mapper、长度和字符限制。
- [ ] 验证不同 ID 类型不可混用，UUIDv7 时间排序不作为安全授权依据。
- [ ] 实现错误到 HTTP/gRPC/Proto status 的独立 mapper。
- [ ] 实现配置 redacted debug，任何 secret 字段输出 `***`。
- [ ] 实现 schema 生成与 example config；example 必须可直接解析。
- [ ] 测试 clock 回拨、deadline 溢出、分页上限、未知配置和错误 source 不泄漏。

```bash
cargo test -p cheetah-signal-types
cargo test -p cheetah-runtime-api
cargo test -p cheetah-config
```

完成条件：基础 crate 无具体 runtime/adapter 依赖，所有公共类型有边界测试和 rustdoc 示例。

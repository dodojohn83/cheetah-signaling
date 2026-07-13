# 05. Protobuf 契约与代码生成

## 1. 文件布局

在 `proto/cheetah/{common,device,control,plugin,media,cluster}/v1/*.proto` 定义六个 package。生成到独立 `cheetah-signal-contracts`，不提交手改生成代码。

## 2. Envelope

`EnvelopeMeta`：message_id、tenant_id、correlation_id、optional causation_id、occurred_at、deadline、source_node_id、owner_epoch、traceparent/tracestate。

`CommandEnvelope`：meta、ResourceRef target、idempotency_key、typed oneof command。`EventEnvelope`：meta、aggregate、aggregate_sequence、typed oneof event。

所有 enum 的 0 值为 `*_UNSPECIFIED`；删除字段必须 `reserved` name/number；map 默认生成 BTreeMap 以获得确定性输出。

## 3. 服务

- `NodeCommand.Execute`
- `ClusterRegistry.RegisterNode/HeartbeatNode/DrainNode`
- `MediaClusterRegistry.RegisterMediaNode/HeartbeatMediaNode/DrainMediaNode`
- `MediaCapability/MediaQuery/MediaRtp/MediaProxy/MediaRecord/MediaSnapshot/MediaControl/MediaEventStream`
- `PluginRuntime.Connect(stream PluginFrame) returns (stream HostFrame)`

为每个 RPC 定义 deadline、idempotency、fencing、错误和 capability 要求。不得用 `Any` 代替核心 command；`Any` 仅用于注册 type URL 的厂商扩展，并限制大小。

## 4. 任务

- [ ] 编写 common IDs、timestamp、error/status、page、resource ref。
- [ ] 编写 device/domain snapshot，不暴露密码和原始报文。
- [ ] 编写 control command/result/event，覆盖 001 v1 全部操作。
- [ ] 编写 media DTO，与媒体 901 类型逐项映射并产出差异表。
- [ ] 编写 plugin hello/welcome/frame、credit、ack、health、drain。
- [ ] 编写 node/media descriptor、lease、capability、load snapshot。
- [ ] 配置 Buf lint/breaking、descriptor set、tonic/prost generation。
- [ ] 编写 domain↔Proto mapper；未知 enum 映射 Unknown/Unsupported，不 panic。
- [ ] 增加 golden binary/JSON fixture 和旧 reader/新 writer 兼容测试。

## 5. 检查

```bash
buf format --diff --exit-code
buf lint
buf breaking --against '.git#branch=main'
cargo test -p cheetah-signal-contracts
```

完成条件：Rust 和至少一个非 Rust 示例 client 能完成握手/command/event roundtrip；连续两次生成无 diff；无复用字段号。

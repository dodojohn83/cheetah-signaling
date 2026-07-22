# GB4-SYS-001：Edge SQLite + fake media 全 GB28181 纵向系统测试报告

## 任务

`GB4-SYS-001`：构建单节点 edge 系统测试，使用 SQLite 存储和 fake/in-memory 媒体，覆盖 GB28181 接入、命令、事件与媒体协商完整路径，且不依赖任何外部服务。

## 范围与边界

- **合成/fake 证据**：存储为本地 SQLite 文件，媒体为 `InMemoryMediaPort`，命令总线为 `InProcessMessageBus`，时钟/ID 为确定性 in-memory 实现。
- **控制面**：测试仅使用 SIP/SDP 控制消息（REGISTER、MESSAGE、INVITE、200 OK、ACK、BYE）和 `MediaPort`/`cheetah.media.v1` 抽象；**不生成、解析、收发或存储任何 RTP/RTCP/PS/TS/ES 媒体负载**。
- **不覆盖**：真实设备/NVR 与真实 media node 互操作（属 `GB4-SYS-003/004`）。

## 实现

| 文件 | 说明 |
|------|------|
| `crates/testing/cheetah-gb-system-tests/Cargo.toml` | 专用集成测试 crate，无生产代码。 |
| `crates/testing/cheetah-gb-system-tests/src/lib.rs` | crate 文档，声明控制面边界。 |
| `crates/testing/cheetah-gb-system-tests/tests/common/mod.rs` | 复用真实 `Gb28181Access`/`Gb28181Media` 状态机的 wire 驱动 helper（REGISTER→401→digest→200、keepalive、catalog、alarm、INVITE/200/ACK/BYE 媒体协商）。 |
| `crates/testing/cheetah-gb-system-tests/tests/gb4_sys_001_edge.rs` | edge 纵向系统测试。 |

测试直接驱动真实协议状态机（不重写解析），再与 application service（`DeviceService`/`MediaService`/`OperationService`/`CommandDispatcher`）和 `ProtocolSessionLink` 组合，保持 protocol module 不直接触达数据库/总线的架构约束。

## 验证的行为

| 环节 | 验证点 |
|------|--------|
| 接入 | wire REGISTER 经 digest 挑战后成功，发出 `DeviceRegistered` 事件 |
| 注册持久化 | application 设备注册 + `ProtocolSessionLink` 持久化 GB28181 协议会话 |
| Keepalive | keepalive 经协议会话记录并保持在线 |
| Catalog 事件 | catalog 响应解析并经 `replace_channel_catalog` 落库，通道上线 |
| Alarm 事件 | alarm 通知解析为事件 |
| 媒体协商 | `Gb28181Media` 完成 INVITE→200(SDP)→ACK，产生 `MediaSessionStarted`；BYE→`MediaSessionStopped` |
| 媒体生命周期 | 经 `MediaService` 启动持久化 live 会话并停止 |
| 命令路径 | PTZ operation 经 `OperationService`→`CommandDispatcher` 派发，本地总线收到并解码 envelope |
| 持久化与恢复 | 重新打开仓储后设备/通道/媒体会话/协议会话状态一致，外部 GB 身份可反查设备 |

## 运行

```bash
cargo test -p cheetah-gb-system-tests --test gb4_sys_001_edge
```

结果：`1 passed`（无需外部服务，无网络访问）。

## 结论

edge 纵向路径在纯 fake/in-memory + SQLite 组合下端到端通过，提供 `GB4-SYS-003/004` 真实互操作之前的确定性基线。

# 03. 生产装配与运行时闭环

## 1. 目标与边界

把已存在的 adapter和service组装成真实edge/cluster进程。assembly只负责配置、依赖注入、listener和生命周期，不承载协议业务。

本章禁止：

- 在assembly中实现领域状态迁移；
- 以空provider或“暂不支持”处理已声明v1能力；
- 配置接受NATS/cluster/plugin/ONVIF但运行时静默忽略；
- readiness早于storage、ownership、media、protocol就绪。

## 2. 配置 profile

### ASM-001：显式 edge/cluster profile

- [x] 增加经校验的deployment profile，不能仅由多个松散bool隐式推导。
- [x] edge默认SQLite + local bus，可连接本机或远程media node；关闭cluster专用依赖。
- [x] cluster要求PostgreSQL + NATS JetStream + ownership；缺少任一配置启动失败。
- [x] 所有listen address、timeout、queue、batch、lease、retry、media endpoint、TLS/secret引用可配置且有上限。
- [x] 未支持的组合在validate阶段返回field violation，不在运行中降级。

证据：[`reports/asm-001-deployment-profile.md`](reports/asm-001-deployment-profile.md)。

### ASM-002：SecretProvider

- [x] assembly先构造SecretProvider，再解析数据库、NATS、TLS、GB和ONVIF凭据引用。
- [x] 移除开发固定digest secret和`NoPasswordProvider`生产路径。
- [x] edge首次启动凭据采用显式bootstrap流程；日志只记录secret ref。
- [x] secret获取失败按required/optional分类，required失败阻止readiness。

证据：[`reports/asm-002-secret-provider.md`](reports/asm-002-secret-provider.md)。

## 3. 基础设施装配

### ASM-003：Storage 与 message bus

- [x] 按backend构造SQLite/PostgreSQL，应用pool size和acquire timeout。
- [x] 按backend构造local/NATS bus，NATS配置stream、durable、max ack pending和dead-letter。
- [x] 启动outbox relay、inbox consumer和poison message处理；退出时有界drain。（outbox + inbox 已启动；poison 路径由 bus NAK/term 处理）
- [x] 不在数据库事务中等待NATS或外部I/O。

证据：[`reports/asm-003-storage-and-bus-assembly.md`](reports/asm-003-storage-and-bus-assembly.md)、[`reports/asm-runtime-closure.md`](reports/asm-runtime-closure.md)。

### ASM-004：Node 与 ownership

- [x] 使用稳定NodeId和每次进程启动的新NodeInstanceId。
- [x] cluster启动registry/heartbeat/assignment/ownership worker；edge使用显式single-owner adapter。（node lease heartbeat + owner renew + edge `SingleNodeOwnerResolver`；assignment 负载均衡仍可增强）
- [x] owner获取原子增加epoch；续租失败撤销设备写权限并停止新命令。（`OwnerLeaseService` acquire/renew；续租失败记日志，inbox 旧 epoch 拒绝）
- [x] takeover、rolling upgrade和shutdown释放采用fencing，不依赖“先下线旧进程”的时序假设。（`TakeoverService` armed；`DrainingMigrationService` 周期扫描 draining peer；node lease cancel→drain）

证据：[`reports/asm-004-node-ownership.md`](reports/asm-004-node-ownership.md)、[`reports/asm-runtime-closure.md`](reports/asm-runtime-closure.md)。

## 4. Media、协议与插件装配

### ASM-005：MediaPort

- [x] 删除主应用`UnsupportedMediaPort`。
- [x] 构造media registry repository、scheduler、typed media client和`SchedulerMediaPort`。
- [x] edge无media node时readiness策略显式配置：`required`阻止ready，`optional`仅允许非媒体API且capability报告unavailable。（`media.readiness_policy`）
- [x] 启动MediaClusterRegistry gRPC server和media event consumers。
- [x] media client证书身份、target node/instance和capability版本在建立调用前校验。（endpoint/scheme、非 nil node id、非 0 instance epoch；TLS identity 参与 pool key；capability 版本协商仍随 media registry 契约演进）

证据：[`reports/asm-005-media-port.md`](reports/asm-005-media-port.md)、[`reports/asm-runtime-closure.md`](reports/asm-runtime-closure.md)。

### ASM-006：GB28181

- [x] 注入SecretStore-backed credential provider、application event sink和device owner router。
- [x] UDP/TCP listener事件进入固定分片runtime，不能只写tracing。
- [x] device upsert、presence、catalog、alarm和command result通过application/transaction/outbox。
- [x] 发送命令路由至当前owner及其protocol session；旧epoch拒绝。（inbox 校验 owner epoch；非本节点转发）

### ASM-007：ONVIF 与 plugin

- [x] 构造ONVIF Tokio driver、discovery worker、provision workflow和bounded device concurrency。
- [x] 将内置GB/ONVIF factory注册到plugin host；启用外部plugin时加载manifest并执行版本/capability校验。
- [x] `ProtocolDriver.handle_command/probe`必须调用真实driver adapter，不得继续返回固定`Unsupported`。（内置 ONVIF/GB instance 启动时 `activate_builtin`；inbox `OwnerCommandHandler` 经 `PluginHost::handle_command` 派发；失败不伪造成功）
- [x] plugin崩溃、健康超时和shutdown不影响其他协议worker。（host 内 instance 隔离；inbox/协议 listener 独立 task + cancel；单 instance 失败仅记日志）

## 5. 生命周期

### ASM-008：启动与关闭

启动固定为：

```text
config/secret
  -> schema/version checks
  -> bus
  -> repositories
  -> node/ownership
  -> media registry/client/events
  -> protocol/plugin workers
  -> internal gRPC
  -> public HTTP
  -> readiness
```

关闭反向执行：

1. readiness=false；
2. 停止新北向请求和新ownership/media reservation；
3. drain protocol/plugin/media callbacks；
4. flush outbox/inbox与审计；
5. 停 listener、worker和repository；
6. 到deadline强制取消并报告未清理资源。

每个spawn task必须由supervisor持有，退出原因进入health；禁止detached task。

## 6. 测试

- edge SQLite/local启动、请求、重启、shutdown。
- cluster配置缺PostgreSQL/NATS/TLS时启动失败。
- local/NATS backend执行同一application smoke。
- media required/optional两种readiness。
- secret缺失、过期lease、NATS断线、media注册晚到、plugin崩溃。
- 连续启动/关闭100次无遗留socket/task。
- 配置中的每个backend/enable字段均有“生效”测试，防止静默忽略。

## 7. 退出门禁

- 主进程不包含生产占位provider或固定开发secret。
- edge和cluster profile均能完成健康启动与有界关闭。
- GB、ONVIF、media、plugin、NATS和ownership配置实际控制运行组件。
- readiness真实反映必需依赖。


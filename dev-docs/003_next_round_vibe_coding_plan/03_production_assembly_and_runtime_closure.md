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

- [ ] 增加经校验的deployment profile，不能仅由多个松散bool隐式推导。
- [ ] edge默认SQLite + local bus，可连接本机或远程media node；关闭cluster专用依赖。
- [ ] cluster要求PostgreSQL + NATS JetStream + ownership；缺少任一配置启动失败。
- [ ] 所有listen address、timeout、queue、batch、lease、retry、media endpoint、TLS/secret引用可配置且有上限。
- [ ] 未支持的组合在validate阶段返回field violation，不在运行中降级。

### ASM-002：SecretProvider

- [ ] assembly先构造SecretProvider，再解析数据库、NATS、TLS、GB和ONVIF凭据引用。
- [ ] 移除开发固定digest secret和`NoPasswordProvider`生产路径。
- [ ] edge首次启动凭据采用显式bootstrap流程；日志只记录secret ref。
- [ ] secret获取失败按required/optional分类，required失败阻止readiness。

## 3. 基础设施装配

### ASM-003：Storage 与 message bus

- [ ] 按backend构造SQLite/PostgreSQL，应用pool size和acquire timeout。
- [ ] 按backend构造local/NATS bus，NATS配置stream、durable、max ack pending和dead-letter。
- [ ] 启动outbox relay、inbox consumer和poison message处理；退出时有界drain。
- [ ] 不在数据库事务中等待NATS或外部I/O。

### ASM-004：Node 与 ownership

- [ ] 使用稳定NodeId和每次进程启动的新NodeInstanceId。
- [ ] cluster启动registry/heartbeat/assignment/ownership worker；edge使用显式single-owner adapter。
- [ ] owner获取原子增加epoch；续租失败撤销设备写权限并停止新命令。
- [ ] takeover、rolling upgrade和shutdown释放采用fencing，不依赖“先下线旧进程”的时序假设。

## 4. Media、协议与插件装配

### ASM-005：MediaPort

- [ ] 删除主应用`UnsupportedMediaPort`。
- [ ] 构造media registry repository、scheduler、typed media client和`SchedulerMediaPort`。
- [ ] edge无media node时readiness策略显式配置：`required`阻止ready，`optional`仅允许非媒体API且capability报告unavailable。
- [ ] 启动MediaClusterRegistry gRPC server和media event consumers。
- [ ] media client证书身份、target node/instance和capability版本在建立调用前校验。

### ASM-006：GB28181

- [ ] 注入SecretStore-backed credential provider、application event sink和device owner router。
- [ ] UDP/TCP listener事件进入固定分片runtime，不能只写tracing。
- [ ] device upsert、presence、catalog、alarm和command result通过application/transaction/outbox。
- [ ] 发送命令路由至当前owner及其protocol session；旧epoch拒绝。

### ASM-007：ONVIF 与 plugin

- [ ] 构造ONVIF Tokio driver、discovery worker、provision workflow和bounded device concurrency。
- [ ] 将内置GB/ONVIF factory注册到plugin host；启用外部plugin时加载manifest并执行版本/capability校验。
- [ ] `ProtocolDriver.handle_command/probe`必须调用真实driver adapter，不得继续返回固定`Unsupported`。
- [ ] plugin崩溃、健康超时和shutdown不影响其他协议worker。

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


# 20 安全、可观测性与运行维护

## 1. 安全基线

威胁面包括未信任 SIP/XML/HTTP 输入、设备弱认证、SSRF、凭据泄露、插件/厂商 SDK、跨租户访问、消息伪造和管理 API 滥用。实现前建立 `security/threat-model.md`，每项威胁关联控制措施和测试。

## 2. 实现任务

### SEC-001：凭据与密钥

- [ ] 定义 `SecretProvider`，实现环境变量、文件和可选外部 secret manager。
- [ ] 数据库只保存 secret reference；确需本地存储时使用 envelope encryption。
- [ ] secret 类型禁止 `Debug/Serialize` 明文输出，临时字节使用 zeroize。
- [ ] 支持凭据轮换的双版本过渡和审计。

### SEC-002：传输与身份

- [ ] 北向 API 和内部 gRPC 生产默认 TLS。
- [ ] 信令节点与媒体节点使用 mTLS 身份，证书映射 node ID。
- [ ] NATS 使用独立账户/权限，只允许所需 subjects。
- [ ] 数据库角色遵循最小权限，迁移角色与运行角色分离。

### SEC-003：输入与资源保护

- [ ] HTTP/SIP/SOAP/XML/Proto 均设置层级和总量限制。
- [ ] 认证前使用更严格的 IP/连接/报文速率限制。
- [ ] 所有出站 URL 经过 SSRF 校验，DNS 解析与连接目标一致性受控。
- [ ] 解压、字符集转换和 XML 扩展均有输出上限。

### SEC-004：审计

审计事件至少包括登录/鉴权失败、设备凭据变更、租户配置变更、设备删除、媒体操作、插件启停、证书/密钥轮换、越权尝试。包含 actor、tenant、action、target、result、request ID、时间；不包含 secret 或完整协议 body。

## 3. 可观测性

### OBS-001：结构化日志

- [ ] 使用 tracing，字段名统一：`tenant_id`、`device_id`、`protocol`、`operation_id`、`command_id`、`media_session_id`、`media_binding_id`、`node_id`、`request_id`。
- [ ] 默认 JSON，边缘交互模式可选紧凑文本。
- [ ] 协议报文日志默认关闭；诊断采样需脱敏、限时、限量并审计。
- [ ] 错误链保留稳定 code 与内部原因，用户响应不暴露内部细节。

### OBS-002：指标

必须提供：

- 注册/在线设备、连接、请求速率与协议错误。
- Operation 各状态数量、端到端延迟、取消、超时和失败码。
- Command/OperationStep 派发、ack、重投、拒绝和积压；不得把 ack 计为业务成功。
- runtime 邮箱、actor、timer、任务和内存。
- DB pool/延迟/冲突，outbox/inbox 积压。
- owner 获取/续租/切换，reconcile backlog。
- 媒体节点健康、调度失败、MediaSession 与 MediaBinding 状态和孤儿资源。
- 插件健康与重启。

高基数字段（device/session ID）禁止作为 Prometheus label，只进入 trace/log。

### OBS-003：分布式追踪

- [ ] HTTP、gRPC、消息 envelope 传播 W3C trace context。
- [ ] SIP/ONVIF 事务创建子 span，并记录安全的协议键。
- [ ] 采样策略保留错误和慢请求，正常高频心跳低采样。
- [ ] 支持 OTLP 导出，导出端故障不阻塞业务。

## 4. 运维接口与手册

- [ ] CLI：配置校验、数据库状态/迁移、节点 drain、设备诊断、outbox 重放、对账触发。
- [ ] `runbooks/`：数据库故障、NATS 故障、媒体节点故障、设备注册风暴、owner 抖动、证书过期、磁盘满。
- [ ] 每个告警包含含义、可能原因、诊断命令、缓解和恢复确认。
- [ ] 支持生成脱敏诊断包，默认排除凭据与原始报文。

## 5. 验收标准

- 依赖审计、许可证检查、secret scan、SAST 和容器扫描进入 CI/发布门禁。
- 故障演练可仅依赖指标、日志和 runbook 定位。
- 自动化测试证明租户越界、SSRF、XML 资源攻击和日志泄密被阻止。

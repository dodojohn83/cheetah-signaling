# 10 北向 HTTP API、鉴权与事件订阅

## 1. 目标

提供稳定、版本化、与协议无关的管理和业务 API。首版使用 Axum 实现 `/api/v1`，OpenAPI 作为可审查契约；异步事件提供 SSE 和 Webhook，WebSocket 可作为兼容扩展，集群内部不依赖北向 API。

## 2. API 范围

资源端点至少包括：

- `/api/v1/tenants`
- `/api/v1/devices`、`/devices/{id}`、`/devices/{id}/channels`
- `/api/v1/operations`、`/operations/{id}`、`/operations/{id}/cancel`
- `/api/v1/media/sessions`、`/media/sessions/{id}`
- `/api/v1/nodes`、`/media-nodes`
- `/api/v1/events/stream`
- `/api/v1/webhooks`、`/webhooks/{id}/deliveries`
- `/health/live`、`/health/ready`、`/metrics`

协议专用诊断信息放在 `/api/v1/devices/{id}/protocol-details`，不能污染通用 Device DTO。

## 3. 通用约定

- JSON 字段使用 `snake_case`；时间使用 RFC 3339 UTC。
- 写请求支持 `Idempotency-Key`，响应返回 `request_id`。
- 乐观更新使用 `If-Match`/ETag 或明确 revision。
- 列表使用不透明 cursor，带稳定排序。
- 错误采用 RFC 9457 Problem Details 风格，包含稳定 `code`，不得暴露内部堆栈。
- API 请求超时不等于 Operation 取消；只有 Operation 与 outbox 成功提交后才能返回 `202 Accepted`。

## 4. 实现任务

### API-001：路由与中间件

- [ ] 实现 request ID、trace、访问日志、超时、body 大小限制、压缩和 CORS。
- [ ] 超时按端点分类，健康检查不得依赖慢外部调用。
- [ ] 提取 `AuthContext { subject, tenant, roles, scopes }`。
- [ ] 在 handler 前执行租户与权限校验。

### API-002：设备与通道 API

- [ ] 创建/更新设备配置，查询在线状态和能力。
- [ ] 列表过滤支持协议、状态、名称前缀和更新时间。
- [ ] 通道目录只读来自设备同步，人工别名作为独立字段修改。
- [ ] 删除设备采用异步退休流程并返回 operation ID。

### API-003：Operation 与媒体 API

- [ ] PTZ、设备重启、目录刷新等创建统一 Operation，内部再派发 typed Command。
- [ ] 开流返回 `202 Accepted`、Operation URL 和 MediaSession reference，或返回幂等命中的原资源；不得等待媒体长期建立。
- [ ] 停流创建引用既有 MediaSession 的 Operation；已停止会话按幂等策略返回原结果。
- [ ] Operation 查询返回权威状态、deadline、result reference 和可公开错误。
- [ ] Command、OperationStep 和 DispatchAttempt 不提供公共查询资源；诊断信息通过受权限控制的 Operation details 暴露。
- [ ] Cancel 将非终态 Operation 标记为取消意图并触发补偿，不把 HTTP 连接断开等同于取消。

### API-004：事件流

- [ ] 客户端可按租户、设备、事件类型过滤。
- [ ] 每事件含单调游标；断线重连可从游标恢复保留窗口内事件。
- [ ] 慢消费者使用有界缓存，超限断开并给出可重连错误。
- [ ] 心跳帧与业务事件区分。

### API-005：Webhook

- [ ] Webhook 配置包含 tenant、目标 URL、事件过滤、secret reference、启用状态和 revision。
- [ ] 使用 event ID、delivery ID 和 timestamp 对原始 body 做 HMAC 签名。
- [ ] connect/request deadline、指数退避、最大次数、熔断和 dead-letter 状态可配置且有上限。
- [ ] 每次 DNS 解析和重定向后重新执行 SSRF 校验，默认拒绝 loopback、link-local、云 metadata 和未允许内网。
- [ ] 投递经持久化队列，失败不能阻塞协议、outbox relay 或 SSE。
- [ ] 提供 delivery 查询与人工重放；同一 delivery ID 重放保持幂等语义。

### API-006：OpenAPI 与 SDK 契约

- [ ] 生成 `openapi/signaling-v1.yaml` 并纳入版本控制。
- [ ] CI 检查实现与文档一致，检测破坏性 schema 变更。
- [ ] 提供 curl 示例和最小 Rust/TypeScript 调用示例。

## 5. 鉴权授权

首版支持静态管理 token（边缘模式）和 OIDC JWT（集群模式）。RBAC 最少定义 `viewer`、`operator`、`tenant_admin`、`system_admin`，具体能力通过 scope 判断。设备协议凭据与北向用户凭据完全分离。

## 6. 测试与验收

- [ ] 每个端点有成功、参数错误、未认证、越权、租户越界和过载测试。
- [ ] 验证 Operation `202`、轮询、取消、超时、幂等命中，以及 Start Operation 成功后 MediaSession 仍为 Active。
- [ ] OpenAPI 示例可作为请求真实执行。
- [ ] 事件重连、慢消费者和游标过期有集成测试。
- [ ] Webhook 签名、DNS rebinding、重试、熔断、死信与重放有集成测试。
- [ ] 敏感字段不出现在响应、日志和 OpenAPI 示例。
- 北向 DTO 不引用任何 SIP/ONVIF XML 内部结构。

# 08. 北向 API、Cluster、Plugin、安全与运维

## 1. PROD-API-001：清除公开占位

扫描所有router、OpenAPI和handler：

- [x] tenant list/create；
- [x] node/media-node list；
- [x] media session get；
- [x] operation入口；
- [x] 其他`NotImplemented`、空page或固定成功入口。

证据：[`reports/prod-api-001-not-implemented-closure.md`](reports/prod-api-001-not-implemented-closure.md)。

每个入口只能选择：

1. 完整实现并测试；或
2. 从v1 router/OpenAPI移除，并在capability声明不支持。

禁止保留可路由的501占位来满足接口数量。

## 2. PROD-API-002：REST 契约

- [x] `/api/v1`使用typed DTO和显式domain mapper（写路径 `JsonBody` → domain DTO）。
- [x] async操作返回202、`Location`（Operation / MediaSession）和可轮询结果。
- [x] 写请求强制`Idempotency-Key`（operation/media create/stop/control）。
- [x] 更新强制ETag/revision（device/webhook PATCH 要求 `If-Match`；GET/更新返回 `ETag`；不匹配 → 412 `FAILED_PRECONDITION`）。
- [x] 列表使用稳定opaque cursor和max page size。
- [x] RFC 9457错误包含稳定code、safe detail、violations和`request_id`（含 JSON 解析失败）。
- [x] path/token tenant 不一致返回403（`extract::resolve_tenant_id`）。

证据：

- [`reports/prod-api-002-rest-contract.md`](reports/prod-api-002-rest-contract.md)
- [`reports/prod-api-002-error-matrix.md`](reports/prod-api-002-error-matrix.md)

测试覆盖：成功路径 + 400 / 401 / 403 / 404 / 412 / 429 Problem Details；
409 / timeout / unsupported / unavailable 仍可按能力补齐。

## 3. PROD-API-003：SSE/Webhook

- SSE支持Last-Event-ID、bounded client queue、gap和慢消费者断开。
- Webhook使用outbox驱动、签名、deadline、有限重试、熔断和dead-letter。
- URL创建及每次连接都执行SSRF/DNS rebinding防护。
- delivery去重、重启恢复和tenant删除清理。
- raw protocol/media body与secret禁止出站。

## 4. PROD-CL-001：Cluster 完整装配

- NATS subject包含环境/tenant/资源分片，定义版本和ACL。
- JetStream durable、ack、redelivery、max delivery和dead-letter固定。
- assignment使用负载/zone/稳定hash，迁移期间保持单owner。
- owner lease、epoch、takeover和rolling upgrade通过三节点测试。
- PostgreSQL/NATS短暂中断后不丢Operation且不产生双副作用。

## 5. PROD-PLUG-001：Plugin 运行时

- built-in和out-of-process plugin使用同一SDK contract。
- Hello/Welcome协商major/minor、capability、frame size和credit。
- command带tenant、deadline、cancel、idempotency和owner epoch。
- secret只用授权handle；plugin identity与mTLS证书匹配。
- crash loop指数退避、熔断；drain/shutdown有界。
- 提供真实example plugin和黑盒contract kit，不以测试dummy代替。

## 6. PROD-SEC-001：认证授权

- edge bootstrap token、static API key、JWT和内部mTLS使用明确profile。
- JWT issuer/audience/algorithm和key rotation验证。
- handler与application均做tenant/scope/resource授权。
- media registry、node command、plugin和message consumer验证内部身份。
- 认证前输入和rate limit更严格。

## 7. PROD-SEC-002：凭据与数据保护

- 所有设备、ONVIF、GB、TLS、DB、NATS和webhook secret只存ref。
- secret类型不实现泄漏Debug/Serialize，临时buffer尽快zeroize。
- 日志/trace/audit/error自动redact Authorization、WS-Security、URL userinfo和SQL参数。
- 建立secret rotation与失效runbook。

## 8. PROD-OBS-001：可观测性

统一日志字段：

```text
tenant_id, device_id, protocol, operation_id, media_session_id,
media_binding_id, node_id, request_id
```

- 高基数ID不作为Prometheus label。
- HTTP/gRPC/NATS/plugin传播W3C trace context。
- metrics覆盖queue、timer lag、owner、outbox/inbox、media RPC、protocol、reconciler和webhook。
- telemetry exporter失败不阻塞业务。
- audit只追加，包含actor/action/target/result/request/time。

## 9. PROD-OPS-001：健康、管理与runbook

- liveness只表示进程事件循环存活。
- readiness检查必需storage/bus/ownership/media/protocol依赖。
- admin drain、reconcile和diagnostic sampling需要system_admin与审计。
- runbook覆盖DB/NATS/media故障、owner抖动、queue saturation、证书/secret轮换、回滚。
- diagnostic采样限时限量，默认不记录raw body。

## 10. 测试与退出门禁

- OpenAPI snapshot/breaking和所有错误矩阵。
- 三节点重复投递、ack丢失、lease过期、rolling upgrade。
- plugin oversized/unknown frame、credit耗尽、崩溃和越权。
- auth跨tenant、资源scope、key rotation和日志泄漏。
- readiness/drain/telemetry失败。
- public router不存在未登记`NotImplemented`。


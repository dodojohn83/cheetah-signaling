# 08. 北向 API、Cluster、Plugin、安全与运维

## 1. PROD-API-001：清除公开占位

扫描所有router、OpenAPI和handler：

- [ ] tenant list/create；
- [ ] node/media-node list；
- [ ] media session get；
- [ ] operation入口；
- [ ] 其他`NotImplemented`、空page或固定成功入口。

每个入口只能选择：

1. 完整实现并测试；或
2. 从v1 router/OpenAPI移除，并在capability声明不支持。

禁止保留可路由的501占位来满足接口数量。

## 2. PROD-API-002：REST 契约

- `/api/v1`使用typed DTO和显式domain mapper。
- async操作返回202、Operation location/ID和可轮询结果。
- 写请求强制Idempotency-Key；更新强制ETag/revision。
- 列表使用稳定opaque cursor和max page size。
- RFC 9457错误包含稳定code、safe detail、violations和request ID。
- path tenant与token tenant不一致返回403且不泄漏资源存在。

测试覆盖成功、400、401、403、404、409、412、429、timeout、unsupported、unavailable和敏感信息。

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


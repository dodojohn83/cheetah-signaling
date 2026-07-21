# GB4-OPS-005：GB28181 运维 Runbook

- 任务：`GB4-OPS-005`
- 状态：`Completed`
- 日期：2026-07-21

## 1. 范围

本 runbook 面向 cheetah-signaling 中 GB28181 协议的 listener、tenant/profile、容量、故障和诊断采样场景，提供可执行的操作步骤、回退命令和检查清单。

## 2. Listener 运维

### 2.1 查看 listener 状态

```bash
curl -s http://localhost:8080/health | jq '.components.gb28181_udp_listener'
```

- `status` 为 `UP`：socket 已 bind，可接收报文。
- `status` 为 `DOWN`：进程或 worker 异常，查看日志中 `listener_id` 和 `local_addr`。

### 2.2 调整 listener 日志采样

诊断采样默认关闭。启用时只记录 method、status code、parser error position、受限 source prefix，不记录 body、Authorization 或密码。

```bash
# 在 runtime_config.yaml 中设置
gb28181:
  diagnostic_sample_rate: 0.001
  diagnostic_max_bytes: 256
  diagnostic_ttl_minutes: 30
```

### 2.3 重启 listener

```bash
# 撤销 readiness，等待 drain 后重启
curl -X POST http://localhost:8080/readyz/disable
curl -X POST http://localhost:8080/healthz/restart/gb28181-udp
```

回退：如重启后注册丢失，检查 `registration_ttl_seconds` 与 SIP `Expires` 是否一致。

## 3. Tenant 与 Profile 运维

### 3.1 验证 tenant 路由

```bash
curl -s http://localhost:8080/api/v1/admin/gb28181/listeners | jq '.[] | {tenant_id, realm, domain, udp_bind}'
```

- `realm` 与 `domain` 必须唯一映射到一个 listener；歧义配置会返回 403/404。
- 旧 `sip_port/sip_domain/default_tenant_id` 配置已被弃用，启动日志中查找 `deprecated_config_converted`。

### 3.2 切换 compatibility profile

```bash
# 仅对指定 device_id 启用 MD5 兼容 profile（默认使用 SHA-256）
curl -X PATCH http://localhost:8080/api/v1/admin/devices/{device_id}/profile \
  -H 'Content-Type: application/json' \
  -d '{"digest_algorithm":"MD5","reason":"vendor_firmware_X"}'
```

要求：必须提供脱敏 fixture、测试用例、风险评估和厂商/型号/固件版本匹配条件。

## 4. 容量与过载

### 4.1 关键指标

| 指标 | 正常范围 | 过载阈值 |
| --- | --- | --- |
| `gb28181_shard_mailbox_depth` | < 100 | ≥ 1000 |
| `gb28181_active_transactions` | < 10k | ≥ 50k |
| `gb28181_timer_lag_seconds` | < 1 | ≥ 5 |
| `gb28181_parse_reject_rate` | < 0.1% | ≥ 1% |
| `gb28181_auth_rate_limited_total` | 基线 | 突增 > 10x |

### 4.2 过载处置

1. 优先确认是否为攻击：查看 per-source rate-limit 触发次数。
2. 对 `REGISTER` flood 启用更严格的 admission：`gb28181.admission_per_source_burst` 减半。
3. 对 `Catalog` 碎片洪流：限制 `catalog_fragment_max_items` 并设置 `catalog_aggregate_timeout_ms`。
4. 扩容：增加 GB28181 shard worker 数量（`gb28181.shard_count` 必须_restart 生效）。

## 5. 故障排查

### 5.1 设备注册不上

检查清单：
- [ ] listener 是否 bind 到正确地址/端口；
- [ ] device_id 是否符合 `gb28181.device_id_pattern`；
- [ ] realm/domain 是否匹配 listener 配置；
- [ ] 设备密码是否已写入 SecretStore 引用 `secret://gb28181/{tenant}/devices/{device_id}`；
- [ ] 检查 `Unauthorized`/`Forbidden` 计数；
- [ ] 抓包采样是否显示 401 质询被重复发送，可能为 nonce 重放或时钟偏移。

### 5.2 媒体启动失败

- 查看 Operation 状态：`GET /api/v1/operations/{operation_id}`。
- 检查 `MediaBinding` 是否 stuck 在 `WaitingForMediaNode`：确认 media node 健康及 owner epoch 是否一致。
- 禁止在 signaling 节点直接绑定 RTP 端口；所有媒体操作由 `MediaPort` 执行。

### 5.3 级联目录丢失

- 检查上级平台 subscription 是否超时；`subscription_refresh_seconds` 是否小于平台要求的 50%。
- 检查 `bridge_loop_detected_total` 是否增长；虚拟目录 ID 与真实 device_id 是否冲突。

## 6. 诊断采样

### 6.1 启用条件

仅在故障定位时启用，不得超过 30 分钟；启用后必须：
- 记录 actor/tenant/action/target/request ID/time；
- 不包含 secret、密码、Authorization、完整 SIP/XML/SDP body、私钥或 SQL 参数；
- 输出到受审计的存储桶，禁止落地到开发机。

### 6.2 脱敏检查命令

```bash
grep -R -E 'Authorization|password|nonce|WWW-Authenticate' /var/log/cheetah-signaling/ || echo "无敏感字段"
```

若命中，立即停用采样并检查 redaction 配置。

## 7. 回退与恢复

- 配置回退：使用 `cheetah-ctl config rollback --version <previous>`。
- 数据回退：SQLite 使用自动快照；PostgreSQL 使用 schema migration 反向后按时间点恢复。
- 灾难恢复：按启动顺序反向执行 shutdown；重启后等待 `readyz` 为 `UP` 再恢复新流量。

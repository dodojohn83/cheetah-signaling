# 07. 安全、可观测与运维恢复

## 1. 目标

确保 GB28181 的公网/专网 listener、Digest、XML/SDP、设备控制、级联出站和高并发状态在攻击、过载、依赖故障和节点重启时具有安全、有界、可诊断的行为。

## 2. 威胁模型增量

至少覆盖：

- 未认证 REGISTER/MESSAGE/INVITE/SUBSCRIBE flood；
- Digest nonce 重放、qop downgrade、algorithm downgrade 和暴力密码尝试；
- realm/domain/DeviceID 混淆导致 tenant 越界；
- Via/Contact/source/rport 欺骗和 endpoint 劫持；
- SIP header CRLF、ambiguous Content-Length、oversized TCP frame；
- XML DTD/XXE、深度/节点/文本炸弹、charset 混淆；
- SDP 地址、端口、payload/attribute 注入和内部网络探测；
- Catalog/RecordInfo/Alarm/位置洪水；
- PTZ/DeviceControl 重放、重复或旧 owner 命令；
- 级联目录泄漏、ID collision、subscription exhaustion 和 bridge loop；
- media callback 伪造或旧 media node instance 推进新 binding；
- 日志、trace、fixture 和错误响应泄漏 secret/原始报文。

新增 profile、outbound endpoint、parser fallback 或鉴权模式时同步更新威胁模型和安全 regression。

## 3. 认证、授权与凭据

- listener 在解析 body 前执行来源/连接/报文大小限流；
- Digest nonce 由注入随机源和 secret 生成，具有 tenant/realm/TTL/context，不记录可重放 material；
- MD5 只允许 compatibility profile 显式开启，默认优先 SHA-256；不得无提示 downgrade；
- 设备密码、平台密码和 digest server secret 通过 SecretStore reference 获取；普通配置和数据库不存明文；
- secret 类型不实现可泄漏 Debug/Serialize，临时 buffer 尽快 zeroize；
- application command 除路由 scope 外再次执行 tenant、RBAC、resource 和 capability 授权；
- platform ACL 限制目录、订阅、control 和 media resource；
- owner epoch、protocol session generation、media instance epoch 在副作用前后都校验。

## 4. 输入与网络策略

### 4.1 SIP/XML/SDP

每个 listener 配置：

- start line/header/body/frame 上限；
- header count/value length、unknown header count；
- XML depth/node/text/item/extension 上限；
- SDP line/media/payload/attribute 上限；
- active transaction/dialog/subscription/aggregation 上限；
- per-source、per-tenant、per-device rate/burst。

XML 禁用 DTD、external entity 和任何外部资源；unknown extension 只保留长度、数量和 schema version 受限的数据。

### 4.2 Endpoint 与出站

- 级联 remote endpoint 必须匹配允许 scheme/transport/port/network zone；
- DNS 解析结果在 connect 前后复验，redirect 默认拒绝；
- Contact/SDP 中的私网、公网和 IPv4/IPv6 地址按 listener/network zone policy 校验；
- signaling 不直接连接 SDP media endpoint，由 MediaPort 决定允许的媒体 node endpoint；
- advertised address 必须显式配置，禁止从不可信 Host/Contact 自动写入公共响应。

## 5. 日志、Trace 与审计

统一结构化字段：

```text
tenant_id device_id protocol operation_id session_id
node_id owner_epoch request_id listener_id transaction_kind outcome
```

禁止记录：

- Authorization/WWW-Authenticate 完整值、密码、nonce server secret；
- 完整 SIP/XML/SDP body、Contact userinfo、RTSP userinfo；
- 未脱敏公网地址、真实位置、私钥或 SQL 参数；
- media credential、handle 中的敏感部分。

允许的诊断信息：method、status code、body bytes、parser error position、header 名、transaction hash、profile ID、受限 source prefix。原始采样必须显式启用、限时、限量、加密保存并产生审计记录。

审计事件覆盖：credential/profile/config 变更、设备 enrollment、平台 link、PTZ/DeviceControl、媒体 Start/Stop、级联目录授权、诊断采样和安全拒绝。审计只追加，记录 actor/tenant/action/target/result/request ID/time，不包含 secret。

## 6. 指标与健康状态

### 6.1 Transport/Core

- UDP datagram/TCP connection 按 listener/transport 统计；
- parse reject reason、frame/header/body size reject；
- active/client/server transaction、retransmission、timeout、duplicate；
- dialog/subscription/timer 数、timer lag；
- shard mailbox depth、processing latency、batch size、rejection/coalescing；
- auth challenge/success/stale/replay/failure/rate-limit。

### 6.2 Application

- register/online/offline/keepalive rate；
- catalog active/fragment/item/partial/timeout；
- command dispatched/succeeded/failed/unknown/cancelled；
- alarm/location accepted/deduplicated/dead-lettered；
- owner conflict/stale epoch rejection；
- media Saga step latency、compensation、orphan binding；
- platform registration/subscription/bridge health。

Device/session/request ID 等高基数值不得成为 Prometheus label。指标 label 限于 listener、transport、method、status class、outcome、profile family 等有界集合。

### 6.3 Readiness/Liveness

- liveness 只表示进程和关键 worker 未死锁；
- readiness 在 listener bind、repository/bus、owner、required media、GB shard 和 secret provider 满足策略后才成功；
- 单个设备/平台失败不使全局 not-ready；所有 shard 停止、队列持续饱和或 required dependency 不可用应 degraded/not-ready；
- health 输出有界摘要，不列出设备 ID 或 secret reference。

## 7. 过载策略

| 过载源 | 策略 |
| --- | --- |
| 新 UDP request | admission/rate limit；可安全响应时 429/503，否则 drop + metric |
| TCP connection/read | 限 connection/permit，stop-read 或关闭慢连接 |
| shard mailbox | 关键事务优先；presence 合并；新低优先工作拒绝 |
| Catalog/RecordInfo | 拒绝新聚合或产生 Partial，不驱逐未知中间片后伪造完整 |
| Alarm flood | durable inbox/outbox + tenant quota；超额明确拒绝/dead-letter |
| DB/NATS 变慢 | bounded in-flight，撤销 readiness 或拒绝新工作，不扩张内存 |
| MediaPort 变慢 | operation deadline/cancel，停止创建新 binding，执行补偿 |
| platform notify backlog | 合并目录快照，关键 Alarm 保序/持久化，超过上限终止 subscription |

过载恢复后 backlog 必须下降到稳态；不得无限重试或同步风暴。

## 8. 启停与恢复

启动顺序遵循：配置/secret → schema → bus → repository → ownership → media → GB runtime/transport → public listener → ready。

关闭顺序反向，并执行：

1. 撤销 readiness；
2. 停止新 enrollment/Operation/command；
3. drain shard/mailbox/transaction 到 deadline；
4. 停止 platform refresh/notify，能安全时注销；
5. cancel media Saga 并保存可恢复 step；
6. cancel timer、关闭 TCP/UDP、释放 permit；
7. drain outbox/repository 后退出。

异常恢复由 inbox/outbox、OperationStep、ProtocolSession、PlatformLink、MediaSession/Binding 和 reconciler 完成；不得依赖进程内 map 作为唯一权威状态。

## 9. 实施任务

- [ ] `GB4-SEC-001`：更新 GB threat model，加入 parser/auth/endpoint/tenant/control/cascade/media callback 风险与测试映射。
- [ ] `GB4-SEC-002`：完成 SecretStore、Digest algorithm/replay、auth rate-limit 和 insecure profile 启动策略。
- [ ] `GB4-SEC-003`：完成 Contact/SDP/级联 endpoint、DNS/network zone 和 redirect policy。
- [x] `GB4-SEC-004`：实现日志/trace redaction、诊断采样和审计事件 contract。见 [reports/gb4-sec-004.md](reports/gb4-sec-004.md)。
- [x] `GB4-OPS-001`：实现 transport/core/shard/application/cascade/media 指标与有界 labels。参见 `dev-docs/004_gb28181-improve/reports/gb4-ops-001.md`。
- [x] `GB4-OPS-002`：实现 GB runtime readiness/degraded、queue saturation 和 dependency health。参见 `dev-docs/004_gb28181-improve/reports/gb4-ops-001.md`。
- [x] `GB4-OPS-003`：实现 admission、priority、coalescing、dead-letter 和 backlog recovery 策略。参见 `dev-docs/004_gb28181-improve/reports/gb4-ops-003.md`。
- [x] `GB4-OPS-004`：实现有界 startup/shutdown/drain 和 crash recovery system test。参见 `dev-docs/004_gb28181-improve/reports/gb4-ops-003.md`。
- [x] `GB4-OPS-005`：编写 listener、tenant/profile、容量、故障和诊断采样 runbook。见 [reports/gb4-ops-005.md](reports/gb4-ops-005.md)。

## 10. 测试与退出门禁

- fuzz/malformed corpus 对 SIP/XML/SDP 不 panic、不越界分配、不泄漏原始输入。
- nonce replay、qop/algorithm downgrade、tenant/realm/body mismatch 和 endpoint hijack 全部拒绝。
- secret、Authorization、完整 body 和高敏地址不出现在 log/trace/error/audit snapshot。
- queue/connection/transaction/dialog/subscription/catalog 每个上限都有触发和恢复测试。
- DB/NATS/SecretStore/MediaPort 降速或短断不会产生无界队列或假成功。
- SIGTERM、worker panic、node crash 和 rolling drain 后资源释放且 reconciler 收敛。
- metrics cardinality 测试证明设备数量增长不增加 label series 的设备维度。


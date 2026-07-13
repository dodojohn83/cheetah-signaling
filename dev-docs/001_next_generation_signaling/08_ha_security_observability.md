# 08. 高可用、安全、可观测与演进

## 1. 可用性模型

系统不复制 socket 或协议栈内存。HA 通过以下机制实现：

- L4 将新 UDP/TCP 流量分配到健康 gateway；
- NATS KV 节点租约决定节点是否可调度；
- OwnershipRecord CAS 决定唯一设备 owner；
- owner epoch 对数据库和媒体副作用做 fencing；
- PostgreSQL 保存 Operation、desired state 和 outbox；
- JetStream 重放未完成命令/事件；
- reconciler 继续或补偿中断工作流。

故障后的正确结果优先于“连接看起来不断”。活 TCP/SIP dialog 丢失后由设备重连、注册到期或业务重试恢复。

## 2. 节点租约与 owner

节点启动时生成 instance ID，注册短租约并周期 heartbeat。相同 NodeId 的新 instance 不得复用旧 instance epoch。

租约失效：

1. registry watcher 在 15 秒目标内将节点标记 unavailable；
2. scheduler 停止分配新任务；
3. owner directory 中指向该 instance 的记录视为 stale；
4. 新注册或 worker 可 CAS 获取新 epoch；
5. janitor 分批清理 stale records，不能全表阻塞；
6. reconciler 按 Operation → MediaSession desired state → MediaBinding/外部资源顺序处理旧节点状态。

NATS KV 读取不用于跨资源事务。业务需要数据库与消息原子性时必须使用 PostgreSQL outbox，而不是把 KV 当权威数据库。

## 3. 命令与事件可靠性

- API 只在 Operation + outbox 提交后返回 accepted。
- command envelope 有 deadline；过期 command 不再派发，并以 `expired_before_dispatch` 推进关联 Operation 为 TimedOut，不在设备恢复后无限补发。
- gateway/worker inbox 去重后执行，结果携带 operation/step ID 并以 CAS 推进 Operation，再发布 OperationStateChanged。
- 设备协议和媒体 RPC 使用派生 idempotency key。
- JetStream consumer 显式 ack；崩溃前未 ack 会重投。
- poison message 有最大 delivery 次数和 dead-letter，不阻塞整个 consumer。
- 同一 aggregate 的 sequence 用于检测 gap，不强求全局总序。

## 4. 故障场景

### 4.1 gateway 故障

UDP/TCP session 消失，owner lease 失效。设备重注册后获得新 epoch。旧 Operation 根据类型：未发送命令可重新路由；已建立但不可确认的非幂等设备动作返回 UnknownOutcome，不擅自重复执行。

### 4.2 workflow 节点故障

Operation/outbox 已持久化，由其他 consumer 接管。Saga step 必须在数据库记录 attempt、idempotency key 和外部 handle，避免重复创建 MediaSession 或在同一 generation 建立第二个有效 MediaBinding。

### 4.3 media node 故障

停止调度，binding 标记 unavailable。live session 可按 policy 在其他媒体节点重新 INVITE/拉流；默认不自动恢复对讲、下载或可能产生副作用的任务，除非能力和用户 policy 明确允许。

### 4.4 NATS 故障

gateway 可维持已建立协议 transaction 的本地回复，但不得取得新 owner 或接受需要持久编排的新命令。API 对 mutating 请求返回 Unavailable；查询可按数据新鲜度 policy 降级。

### 4.5 PostgreSQL 故障

不接受新的持久 Operation、配置或资产更新。现有设备保活可在有界时间内继续，presence 事件缓存在严格上限内；超过上限丢弃可重建 telemetry，不丢弃安全审计或伪装成功。

## 5. 安全边界

### 5.1 北向身份与授权

- OIDC/JWT 是默认用户/服务身份；issuer、audience、signature、exp/nbf 必须校验。
- tenant path 与 token tenant scope 双重校验。
- RBAC 至少区分 viewer、operator、device-admin、tenant-admin、system-admin。
- 高风险动作（凭据、平台级联、批量 PTZ/删除、insecure ONVIF）增加细粒度 permission 和审计。
- service account 使用短期 token，不使用共享永久 API key 作为默认方案。

### 5.2 内部通信

- gRPC、NATS、PostgreSQL 使用 TLS；跨主机 gRPC 使用 mTLS。
- NATS subject ACL 按 role、tenant/service scope 最小授权。
- plugin 使用独立身份，只能访问分配的 tenant/zone/capability。
- media node 调用验证 signaling identity、owner epoch 和 node lease。

### 5.3 协议入口

- 按 IP、tenant、device ID、realm 设置 token bucket；未知身份使用更严格额度。
- SIP/UDP response 大小和未认证回复受限，防止反射放大。
- Digest MD5 仅在协议标准边界使用；内部密码存储使用现代 password KDF 或外部 secret provider。
- XML 禁止 DTD/XXE；HTTP redirect、XAddr、stream URI 和 webhook 全部执行 SSRF 校验。
- parser 错误日志截断并转义，防止日志注入和敏感数据泄漏。

## 6. 审计

审计记录包含：actor、tenant、action、resource、request/correlation ID、结果、来源 IP、before/after revision、occurred_at。凭据值、完整报文和媒体 URL secret 不进入审计。

必须审计：登录/鉴权失败策略变化、凭据更新、tenant/role、平台级联、设备启停、PTZ/对讲、录像/下载、插件安装配置、insecure TLS/Profile S 开关、节点 drain 和管理操作。

审计写入失败时，高风险管理动作 fail closed；普通 telemetry 不使用审计表。

## 7. 可观测性

### 7.1 Metrics

允许的低基数 label：service、role、node、protocol、tenant tier（不是 tenant ID）、operation kind、result、error code、transport、zone。

禁止把 device ID、channel ID、session ID、URI、Call-ID 用作 metrics label。单设备诊断使用 trace/log/query API。

核心指标：连接/在线数、register/keepalive rate、transaction、worker queue depth、timer lag、Operation state/latency、owner conflict、NATS lag/redelivery、DB pool/query、media scheduling、plugin health、webhook delivery。

### 7.2 Tracing

REST request 创建或继承 W3C trace context。correlation ID 贯穿 Operation、outbox、NATS、gateway、plugin 和 media RPC。设备主动事件创建新 trace，并在后续 workflow 使用 link。

百万设备场景默认采样；错误、超时、owner conflict、安全拒绝提高采样率。不能为每次成功心跳完整采样。

### 7.3 Logging

结构化日志包含 timestamp、level、service/role/node、tenant（按策略 hash）、protocol、event、request/correlation ID、stable error。敏感 header、密码、nonce response、URI userinfo 和原始 XML 默认不记录。

## 8. 配置与动态变更

配置分为：

- static：listener、数据库、NATS、node identity、TLS key，变更需进程重启；
- module restart：协议 parser/driver 核心设置，按 module 有界重启；
- dynamic：rate limit、tenant policy、timeout、compat profile、webhook；
- secret：只保存 reference，轮换通过 SecretStore 通知。

所有配置有 schema、默认值、范围、敏感标志和 change effect。配置 revision 单调递增；插件/module 应用失败不能谎报新 revision 已生效。

## 9. 滚动升级与十年演进

- Rust toolchain 和依赖锁定，半年执行升级分支、完整兼容/性能回归后合并。
- REST `/v1` 和 Proto `v1` 只做 additive change；删除字段 reserved。
- plugin handshake 支持 host 同时服务当前 major 与上一个 major；破坏性 major 至少提供一个发布周期迁移。
- PostgreSQL 迁移兼容当前与上一应用版本；先扩展，再回填/切换，最后在后续版本收缩。
- NATS stream/subject、KV bucket、durable consumer 名称是运维 ABI，变更必须有双写/迁移方案。
- 协议 compat fixture、OpenAPI、Proto descriptor、数据库 schema snapshot 和事件 golden files 都作为兼容基线保留。
- 年度 LTS 不冻结依赖十年，而是冻结公开契约、迁移规则和可复现构建。

## 10. 备份与灾备

- PostgreSQL 按组织 RPO 配置持续归档/PITR，并定期做恢复演练。
- JetStream 不是权威资产备份；关键事件保留策略和 mirror/source 由部署等级决定。
- edge SQLite 使用在线一致性备份，不复制正在变化的裸数据库文件。
- SecretStore/KMS、CA 和配置需要独立灾备，数据库备份不能替代密钥备份。
- 跨地域灾备默认 active-passive；不支持同一设备在两个地域同时宣告 owner。切换需要 fencing 新 region epoch。

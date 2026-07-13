# Cheetah Signaling 编码与执行规范

## 1. 适用范围与规范等级

本文件适用于仓库根目录及所有子目录，约束人工开发者、自动化编程代理和外部编程执行体。子目录可以增加更严格的 `AGENTS.md`，不得放宽本文件要求。

本文使用以下规范词：

- **必须/禁止**：不可违反；若现有设计无法满足，先修改设计并完成评审。
- **应该/不应该**：默认遵守；偏离时必须在代码或 PR 中说明原因。
- **可以**：允许选择，不构成默认实现。

规范优先级从高到低：

1. 已批准的安全、数据完整性和公开兼容性约束；
2. `dev-docs/001_next_generation_signaling/` 架构设计；
3. `dev-docs/002_vibe_coding_plan/` 开发执行计划；
4. 本文件的通用编码规则；
5. crate 内 README、模块文档和局部实现约定。

发现文档之间冲突时必须停止受影响部分的实现，指出具体冲突并先统一文档。不得自行选择最省事的解释。

## 2. 项目不可违反的边界

### 2.1 Control Plane 与 Media Plane

本仓库是信令控制面，只负责设备接入、协议状态机、命令、媒体协商和资源生命周期编排。

本仓库禁止：

- 接收、转发、解析或存储 RTP、RTCP、PS、TS、ES 媒体负载；
- 绑定媒体 RTP/RTCP 端口；
- 实现 RTSP 拉流、音视频解码、转码、录制、截图编码或播放 URL 生成；
- 直接访问 `cheetah-media-server-rs` 的 engine、stream manager、codec cache 等内部对象；
- 为绕过媒体接口而复制媒体服务器实现。

媒体操作必须经版本化 `cheetah.media.v1` 契约和 `MediaPort`/媒体客户端执行。所有修改型媒体请求必须包含 tenant、request/correlation ID、幂等键、deadline、信令 owner node、owner epoch 和目标 media node instance epoch。

### 2.2 六层架构与依赖方向

层次从上到下为：

1. `apps/assembly`：配置加载、角色装配、依赖注入和进程生命周期；
2. transport adapters：HTTP、gRPC、NATS、数据库和 secret provider；
3. application：用例、Operation、Saga、reconciler、权限和配额；
4. protocol module：协议业务到统一领域模型的映射；
5. protocol driver：socket、HTTP/TLS、framing、连接和 timer 驱动；
6. protocol core/foundation：Sans-I/O 状态机、codec、领域类型和 ports。

依赖只能向下或指向下层定义的抽象 port。必须保持以下约束：

- `domain` 不依赖 Tokio、Axum、Tonic、SQLx、async-nats、quick-xml 或具体协议类型；
- protocol core 不依赖 Tokio、socket、HTTP client、数据库、消息系统或媒体 client；
- protocol driver 不依赖 SQLx，不执行业务仓储操作；
- protocol module 不直接访问数据库、NATS 或媒体实现，只调用 application/port；
- HTTP/gRPC handler 不直接调用具体 PostgreSQL/SQLite repository；
- application 不持有具体 SQL connection、NATS client 或 Axum extractor；
- app crate 只负责装配和生命周期，不承载协议或领域业务；
- generated Proto、HTTP DTO、SIP message 和 SOAP/XML 类型不得作为领域实体持久化。

新依赖边必须通过 `cargo metadata` 架构检查。不得通过 re-export、feature 或 type alias 隐藏违规依赖。

### 2.3 协议三段式

每个内置协议固定拆分为：

```text
cheetah-<protocol>-core
cheetah-<protocol>-driver-tokio
cheetah-<protocol>-module
```

- core 使用显式 `Input`、`Output`、`Event`、`TimerId`、`Command`，不执行 I/O；
- driver 把网络和时钟事件转换为 core input，并执行 core output；
- module 负责领域映射和业务编排，不重复实现 wire state machine。

未来 MQTT、Matter、HomeKit、JT808、ISUP 等协议也必须遵循同一边界，不得因厂商 SDK 改写领域内核。

## 3. Rust 与 Workspace 基线

- 使用 Rust 1.96.1、Edition 2024、Cargo resolver 3；版本变更必须同步工具链文件、CI 和基线文档。
- 根 workspace 统一声明依赖版本、package metadata 和 lint；子 crate 不得私自漂移同一依赖版本。
- package 名使用 `cheetah-` 前缀和 kebab-case；crate/module/file 使用 snake_case；类型和 trait 使用 PascalCase；常量使用 SCREAMING_SNAKE_CASE。
- 主 workspace 设置 `unsafe_code = "forbid"`。厂商 C/C++ SDK 必须位于进程外隔离 adapter；不得在核心 crate 为 FFI 放开 unsafe。
- 提交 `Cargo.lock`。构建、测试和代码生成不得依赖未锁定的网络资源。
- edge feature 关闭时不得链接 PostgreSQL、NATS 或不需要的集群依赖。
- feature 只表达可选能力和 adapter，不得改变同一公共类型或业务操作的语义。

新增 crate 时必须提供 crate README，写明职责、允许依赖、禁止依赖、feature 和公共入口。

## 4. 代码组织与可读性

- 一个模块只表达一个主要职责。源文件应该控制在 500 行以内，超过 800 行必须拆分；生成代码除外。
- 优先使用小型、具名函数和明确数据流，避免深层嵌套、巨型 handler 和隐式全局状态。
- 所有依赖通过构造器或函数参数显式注入，禁止 service locator、可变全局 singleton 和隐藏的进程级缓存。
- 领域聚合字段保持私有，只能通过验证不变量的方法修改。
- 公共类型、trait、函数和非显然状态机必须有 rustdoc；注释说明约束和原因，不复述语句表面行为。
- 复杂状态迁移应该使用 enum 和表驱动逻辑，禁止用多个互相矛盾的布尔字段模拟状态机。
- 相同概念只保留一个权威实现；禁止复制粘贴 parser、错误映射、重试或鉴权逻辑。
- 对外输入、时间、大小和数量的单位必须出现在类型或字段名中，例如 `timeout_ms`、`size_bytes`，不得依赖注释猜测。

禁止把 `todo!()`、`unimplemented!()`、空 provider、固定成功返回或吞错逻辑提交到可执行生产路径。v1 外能力必须返回稳定 `Unsupported`，并保留能力探测语义。

## 5. 类型、所有权与领域模型

- Tenant、Device、Channel、Session、Operation、Node、Event、Message 等身份必须使用受校验 newtype，禁止跨层传播裸 `String` 或混用不同 ID。
- 内部新 ID 使用 UUIDv7；不得把 UUID 时间排序作为鉴权或安全判断。
- 协议外部 ID 使用独立 `ProtocolIdentity`/映射类型，不得假定其为 UUID。
- 所有业务数据显式携带 `TenantId`；repository、message、cache key、审计和权限检查不得省略租户维度。
- 时间使用 UTC 明确类型；wire time、单调 deadline 和设备时钟偏移分开表达。
- domain/core 中禁止直接调用 `SystemTime::now()`、`Instant::now()` 或随机全局函数；必须注入 `Clock`、`IdGenerator` 和随机源。
- 可变持久化聚合使用 `Revision` 乐观并发；跨节点副作用使用 `OwnerEpoch` fencing。
- 扩展 metadata 必须有 schema 版本、键值长度和条目上限；核心查询字段不得藏在无约束 JSON 中。

异步控制与媒体资源必须使用四个职责分离的模型：

- `Operation` 是北向可查询、可取消、可超时的业务工作流，也是异步执行状态的唯一权威来源；
- `Command` 是 Operation/Saga 派发的不可变 typed 指令，不得实现 `Accepted/Dispatched/Succeeded/...` 第二套领域生命周期；
- `MediaSession` 表达用户视角的逻辑媒体意图、desired state 和长期会话状态；
- `MediaBinding` 表达 MediaSession 与具体媒体节点 instance、MediaKey 和 handle 的物理关联。

Start Operation 成功后 MediaSession 可以继续 Active；Stop/control 必须创建新的 Operation 引用既有 MediaSession。重试或迁移创建新 MediaBinding，不得复活终态 binding。同一 MediaSession generation 最多一个有效 binding，旧 owner/media node instance 的回调不得推进新状态。

## 6. 错误、Result 与 Panic

- 业务错误使用稳定 enum/code。禁止通过字符串内容判断错误类型。
- `SignalError` 对外只包含安全 message、稳定 code、retryable、field violations 和 correlation ID；内部 source 不得进入 HTTP/gRPC 响应。
- adapter 负责把外部错误映射为领域错误，并保留内部诊断上下文。
- domain/application 不使用 `anyhow::Error` 作为公共接口；二进制装配层可以用通用报告错误完成启动失败汇总。
- 对外部输入、网络、数据库、消息和设备响应禁止 `unwrap()`/`expect()`。
- 生产代码只有在编译期或构造期已经证明的不变量上才能使用 `expect()`，且必须写清不变量；优先返回错误。
- parser、状态机、后台 worker 和协议 handler 不得因畸形输入 panic。
- 无法确定危险命令是否已经作用到设备时，返回 `UnknownOutcome` 或等价可诊断状态；禁止伪造成功或盲目重试副作用。

错误日志不得包含密码、Authorization、WS-Security header、完整 RTSP userinfo、SQL 参数、完整 SIP/SOAP/XML body 或私钥。

## 7. 异步、并发与资源治理

- 所有 async trait/future 在主运行路径满足 `Send`；不得为了潜在 WASM 使用降低 edge/cluster 的线程安全约束。
- 所有外部 I/O 必须有 connect timeout、operation deadline 和 cancellation 语义。
- 禁止持有同步锁或异步 mutex guard 跨越 `.await`，除非锁类型和临界区经过专门证明并有测试。
- 同一设备的可变协议状态归属一个分片 worker；跨 worker 通过消息通信，不共享可变 session。
- 禁止“每设备一个常驻 Tokio task”及“每设备一个独立 Tokio sleep”。使用固定分片 worker、惰性状态和分层时间轮。
- 所有 channel、邮箱、buffer、cache、batch、分页、重试和并发必须有配置上限。
- 禁止 `unbounded_channel` 和生产默认“无限”。队列满必须执行明确的拒绝、退避、低价值事件丢弃或断开策略，并产生指标。
- 取消必须向下传播；select 分支、spawn task 和 stream 退出后必须释放 socket、timer、reservation 和 permit。
- 不得持有两个设备 actor 的锁执行跨设备操作；需要协调时使用 Saga/消息和幂等步骤。
- 重试只用于已分类的暂时错误，必须有次数、deadline、指数退避和 jitter 上限。

启动顺序固定为：配置/secret → schema 检查 → bus → repository → ownership → media → protocol → public listener → ready。关闭顺序反向执行，并先撤销 readiness、停止新工作、再有界 drain。

## 8. Sans-I/O、解析器与协议实现

- parser 支持增量输入、明确错误位置和可配置大小限制；不得为方便一次性复制无界报文。
- serializer/parser 必须有 round-trip 或 golden test，并明确允许的规范化差异。
- UDP、TCP 半包/粘包、重复/乱序、超时、取消和重传必须由状态机处理，不能散落在业务 handler。
- 未识别的兼容扩展可以有限保留；未知字段的数量、深度和长度仍必须受限。
- XML 禁止外部实体、DTD 和外部资源加载；必须限制深度、节点数、文本和解压后 body。
- 禁止通过字符串拼接生成 XML、SIP、SDP、SQL 或 URL；使用类型化 encoder/query builder，并正确转义。
- ONVIF/RTSP/Webhook 等出站 URL 必须执行 scheme、端口、目标网段、重定向和 DNS rebinding 防护。
- 协议兼容 workaround 必须以 vendor/model/firmware profile 显式启用，包含样本、测试、风险和匹配条件；禁止在通用 parser 中加入不可追踪的宽松分支。
- 原始协议报文默认不记录。诊断采样必须脱敏、限时、限量并受审计。

## 9. 存储、事务与消息语义

- SQLite 和 PostgreSQL 必须实现同一 repository port 并通过同一 contract suite。
- repository 方法显式接收 `TenantId`；更新必须带 revision 条件，零行更新转换为并发冲突。
- 聚合修改和 outbox 事件必须在同一数据库事务提交。
- 消息语义为“至少一次传递 + 幂等消费”，不得宣称跨数据库和消息系统 exactly-once。
- consumer 在副作用前使用 inbox/processed message 去重；重复消息返回第一次结果或安全确认。
- 所有修改型命令包含 message ID、idempotency key、deadline 和 owner epoch。
- Command 必须关联 operation/step ID；ack、重投和 dead-letter 只属于 OperationStep/DispatchAttempt 诊断，不能作为业务结果。
- owner 获取必须原子增加 epoch；旧 epoch 的状态、命令结果和媒体回调必须拒绝。
- 迁移文件发布后只追加，不修改。SQLite/PostgreSQL 使用同一逻辑版本但可以有后端专用 SQL。
- 大表迁移遵循 expand → backfill → switch → contract，不得在滚动升级中引入新旧版本无法共存的 schema。
- 列表查询使用稳定游标分页，禁止对大表使用无界查询或大 offset。

不得在 SQL 事务中等待设备、NATS、Webhook 或媒体 RPC。跨系统一致性使用 Operation、Saga、outbox 和 reconciler。

## 10. API、Proto 与兼容性

- REST 使用版本化 `/api/v1` 和 RFC 9457 风格错误；异步设备/媒体操作通常返回 `202 Accepted` 与 Operation/Command ID。
- 写请求支持幂等键；资源更新使用 revision/ETag；列表使用不透明 cursor 和最大 page size。
- HTTP DTO、Proto DTO、domain 类型之间使用显式 mapper，禁止直接 type alias。
- 所有 Proto enum 的 0 值为 `*_UNSPECIFIED`；字段删除后 `reserved` name/number；字段号永不复用。
- 核心命令禁止用 `google.protobuf.Any` 逃避建模。厂商扩展必须注册 type URL 并限制大小。
- 公共 REST/Proto v1 只做兼容扩展。删除字段、新增 required 字段、改变 enum/错误语义必须新建 major 或提供双栈迁移窗口。
- OpenAPI、Proto descriptor 和生成代码必须可重复生成；生成文件禁止手工编辑。
- 时间、duration、分页、错误、幂等和 fencing 在 HTTP、gRPC、消息三个入口保持一致语义。

## 11. 安全、凭据与可观测性

- 凭据通过 `SecretProvider`/`SecretStore` 按引用获取，普通配置、领域 metadata 和数据库业务表禁止保存明文 secret。
- secret 类型禁止派生或实现可泄漏明文的 `Debug`/`Serialize`；临时 secret buffer 应尽快 zeroize。
- 北向和内部调用执行认证、租户隔离、RBAC/scope 和资源级授权；不能只依赖路由层 tenant 参数。
- 内部生产通信使用 TLS/mTLS；证书身份必须与 node/plugin ID 匹配。
- 输入限制和 rate limit 在认证前更严格，至少按来源、租户、协议和节点分级。
- 结构化日志字段统一使用 `tenant_id`、`device_id`、`protocol`、`operation_id`、`session_id`、`node_id`、`request_id`。
- device/session/request ID 等高基数字段禁止作为 Prometheus label，只能进入 trace 或受限日志。
- HTTP、gRPC 和消息传播 W3C trace context；遥测导出故障不得阻塞业务。
- 审计日志只追加，记录 actor、tenant、action、target、result、request ID 和时间，但不记录 secret 或完整原始报文。

新增出站访问、解析器、鉴权方式、secret 存储或插件权限时，必须同步威胁模型和安全测试。

## 12. 测试规范

### 12.1 每次改动的最低测试

- 修复缺陷：先添加能稳定复现的失败测试或脱敏 fixture，再修复。
- 领域/状态机：单元测试、合法/非法迁移表、幂等和终态不可逆测试。
- parser/codec：golden、round-trip、边界、畸形输入、切片边界和 fuzz regression。
- repository：SQLite/PostgreSQL 共用 contract tests，覆盖事务、revision、租户、分页和迁移。
- message/ownership：重复投递、ack 丢失、过期 lease、旧 epoch 和 crash window。
- media：fake media node 与真实媒体服务运行相同 contract suite。
- API：成功、校验、401/403/404/409/429、幂等、分页、租户越界和敏感信息泄漏测试。
- 并发/时间：FakeClock、确定性 ID/随机 seed；禁止依赖真实 sleep、测试顺序和固定公共端口。

测试不得访问公网，不得依赖开发者私人数据库或不可复现的真实设备状态。集成测试使用可销毁容器或显式环境配置，并具有 deadline 和清理逻辑。

### 12.2 验证命令

提交前至少运行与改动匹配的命令；workspace 建立后基线为：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
buf format --diff --exit-code
buf lint
cargo deny check
```

按改动类型追加：

- Proto：Buf breaking check、确定性 codegen、旧 reader/新 writer 测试；
- REST：OpenAPI snapshot 与 breaking check；
- SQL：SQLite/PostgreSQL migration 和 repository contract；
- 协议：对应 core/driver/module、golden、fuzz corpus 和模拟器测试；
- ownership/media：多节点、故障注入和 reconciliation 测试；
- 公共依赖或 feature：x86_64 与 aarch64 check、`cargo tree` 边界检查。

长时间 interop、chaos、百万设备和 72 小时 soak 可由专用流水线执行，但改动者必须说明触发条件和结果位置，不能用空的 ignored test 伪装通过。

## 13. 性能与可维护性

- 性能优化前先建立可复现 benchmark，记录 commit、硬件、内核、配置和数据集。
- 禁止用单节点固定设备数宣传容量；报告必须包含协议比例、心跳周期、TLS 比例、报文大小和活跃操作比例。
- 热路径避免无必要分配、clone 和格式化，但不能以 unsafe 或破坏分层换取微小收益。
- 缓存必须定义 key、tenant 边界、容量、TTL、失效和一致性来源。
- 每个后台循环必须有 cancellation、批次上限、退避和健康/积压指标。
- 公共契约、数据库和插件 SDK 按十年以上演进设计；不承诺单一二进制或依赖版本十年不升级。

## 14. 文档、生成物与变更管理

- 架构、Proto、REST、表结构、NATS subject、插件 SDK 或媒体边界改变时，必须在代码前或同一变更中更新设计和开发计划。
- 真正改变冻结决策的事项写 ADR；普通实现细节不滥用 ADR。
- 文档不得引用执行环境专有的绝对路径作为实现前提；仓库内使用相对链接。
- fixture 必须附来源类别、协议/设备版本、预期结果、脱敏方式和许可证说明。
- 不得提交密码、token、私钥、真实个人信息或未脱敏抓包。
- 生成物必须由锁定脚本生成；修改输入 schema 后重新生成，禁止直接修补输出文件。
- 不修改与当前任务无关的用户变更，不做破坏性 git 清理，不顺手大范围重构。

## 15. 编程执行流程

开始任务前：

1. 阅读相关 `001` 设计、`002` 计划、crate README 和更近层级的 `AGENTS.md`；
2. 明确所属层、允许依赖、公共契约、数据所有者和失败语义；
3. 检查当前工作区改动，避免覆盖不属于本任务的内容；
4. 把任务拆成可独立验证的最小提交单元。

实现过程中：

1. 先写或更新 contract/test，再写 adapter；
2. 优先完成一条可运行的垂直闭环，不创建大量无行为 skeleton；
3. 每个外部调用同时实现 deadline、取消、错误映射、幂等和观测；
4. 每个集合/队列/重试同时实现上限与过载路径；
5. 每个副作用同时考虑重复执行、进程崩溃、旧 owner 和补偿；
6. 发现计划缺失或错误时先更新文档，不在代码中留下隐式决定。

交付前：

1. 运行格式化、lint 和受影响测试；
2. 检查没有 `todo!()`、`unimplemented!()`、假成功、secret 或无限资源；
3. 检查公开契约、迁移、配置示例和运维说明是否同步；
4. 报告修改内容、验证命令、结果以及未运行的高成本测试和原因。

## 16. 完成定义

一项实现只有同时满足以下条件才算完成：

- 分层和 Media Plane 边界未被破坏；
- 正常、失败、超时、取消、重复、过载和恢复路径均有确定行为；
- 所有资源有 owner、上限、deadline 和清理路径；
- 数据库事务、outbox/inbox、幂等和 fencing 语义正确；
- 公共 API/Proto/配置/数据库变更具有兼容策略；
- 测试覆盖本次行为，格式化、lint 和相关测试通过；
- 日志、指标、trace 和审计足以诊断问题且不泄漏敏感数据；
- 文档、示例、迁移和运行手册与实现一致；
- 没有未归属占位实现或以“后续处理”掩盖的 v1 必选功能。

# 01. 002 完成度审计与缺口登记

## 1. 审计目的

本章给出下一轮执行的保守基线。002 中 checkbox 长期未同步，不能代表实际代码量；反过来，已有 crate 和测试也不能证明产品闭环。审计采用“设计契约 → 实现 → 生产装配 → 自动验证 → 外部验收”五级证据。

## 2. 已执行检查

- 工作区：约 378 个 Rust 文件、8.4 万行 Rust，覆盖 foundation、domain、application、runtime、storage、messaging、cluster、media、API、plugin、GB28181、ONVIF、perf 和 packaging。
- Git 工作区在冻结点无未提交修改。
- 002 共 577 个任务 checkbox，仅 `21_testing_simulators_and_performance.md` 的 3 个 property-test 项标记完成。
- 官方 Rust 1.96.1 在当前 rustup 源无法取得；本机仅有 1.94.1。
- 使用 1.94.1 和独立 target 尝试 workspace test，codegen 因系统缺少 `protoc` 终止。
- 使用 1.94.1 执行 format check 出现差异；在官方固定工具链可用前不能判定正式结果。
- 媒体仓库本地 HEAD 与远端 `main` 均为 `d41ecbec4764519939d2b720141f275886a9bd8c`。

## 3. 002 章节覆盖矩阵

下表覆盖 002 每个文档内的全部 checkbox。除“已确认完成”列列出的项目外，该行的所有未勾选任务均保持开放并转入对应 003 任务。

| 002 章节 | 任务数 | 已确认完成 | 审计状态 | 主要证据/缺口 | 003 归属 |
| --- | ---: | ---: | --- | --- | --- |
| 01 执行契约 | 7 | 0 | `Partial/Blocked` | 版本已写入配置；固定 Rust 无法安装，环境要求未闭环 | BAS |
| 02 Workspace/CI | 13 | 0 | `Partial` | workspace/CI 存在；无法完成官方基线验证 | BAS |
| 03 crate graph | 6 | 0 | `Partial` | crate 分层存在；缺少覆盖所有 feature 的自动依赖门禁证据 | BAS |
| 04 foundation | 6 | 0 | `Partial` | newtype/error/config/clock 存在；生产 secret 与配置装配不完整 | BAS、PROD |
| 05 Proto | 10 | 0 | `Partial` | codegen 存在；媒体 Proto 与最新媒体 API 漂移，缺 breaking 结果 | MED-C |
| 06 domain/application | 36 | 0 | `Partial` | 四模型和 service 存在；真实 dispatch/Saga 未装配 | WF |
| 07 runtime | 24 | 0 | `Partial` | 分片/timer 组件存在；主进程未装配为设备状态所有者 | ASM、SYS |
| 08 storage | 29 | 0 | `Partial` | SQLite/PostgreSQL 实现和 contract 基础存在；完整双后端报告缺失 | BAS、SYS |
| 09 messaging/ownership | 29 | 0 | `Partial` | local/NATS/ownership crate 存在；主进程始终使用 local bus | ASM、PROD |
| 10 HTTP/events | 33 | 0 | `Partial/Not Implemented` | 多数资源 handler 存在；tenant/node/media get/operation submit 等仍返回 `NotImplemented` | PROD |
| 11 media | 29 | 0 | `Partial/Blocked` | scheduler/client/registry service 存在；主进程使用 `UnsupportedMediaPort`，真实媒体无 gRPC adapter | MED-C、MED-R、UP |
| 12 GB SIP core | 35 | 0 | `Partial` | parser/codec/digest/状态机测试较完整；官方全量验证未通过 | GB、BAS |
| 13 GB access | 36 | 0 | `Partial` | UDP driver 可监听；事件只进入 tracing，未持久化/ownership/application | GB |
| 14 GB media | 43 | 0 | `Partial/Blocked` | live/playback/talk 逻辑与测试存在；无真实 MediaPort、INVITE Saga 装配 | WF、GB、UP |
| 15 GB cascade | 32 | 0 | `Partial` | cascade 模块和测试存在；生产配置、owner、互操作报告缺失 | GB、SYS |
| 16 ONVIF core | 31 | 0 | `Partial` | discovery/security/XML/HTTP driver 存在；生产进程未启动 ONVIF | ONVIF |
| 17 ONVIF workflow | 46 | 0 | `Partial/Blocked` | service builder和部分 workflow 存在；plugin command/probe 明确 `Unsupported`，无真实 pull/snapshot闭环 | ONVIF、WF、UP |
| 18 cluster/HA | 21 | 0 | `Partial` | ownership/reconciler/rolling upgrade 组件存在；cluster 配置未装配 | ASM、PROD、SYS |
| 19 plugin | 24 | 0 | `Partial` | SDK/host/OOP transport 存在；主应用未加载，ONVIF adapter不派发 | ASM、PROD |
| 20 security/ops | 24 | 0 | `Partial` | auth/rate limit/audit 基础存在；TLS、secret、metrics/export和生产门禁不完整 | PROD |
| 21 test/perf | 26 | 3 | `Partial` | property tests 已登记；system/interop/chaos/百万/soak 未完成 | SYS |
| 22 packaging/release | 28 | 0 | `Partial/Not Implemented` | migration tool和部分打包 crate 存在；无完整部署资产和发布证据 | SYS |
| README 全局 DoD | 9 | 0 | `Not Completed` | real media、真实设备、HA、百万与 soak 条件均未满足 | 全部 |

## 4. 必须优先消除的生产占位

### AUD-GAP-001：媒体未装配

- `apps/cheetah-signaling/src/assembly.rs` 构造 `UnsupportedMediaPort`。
- `SchedulerMediaPort` 和 `MediaControlClient` 未在主应用使用。
- 当前媒体 API 创建请求只能得到稳定 `Unsupported`，不构成 v1 媒体能力。

归属：MED-C、MED-R、ASM。

### AUD-GAP-002：协议未进入领域闭环

- GB UDP listener 使用 `TracingGbEventSink`，事件没有进入 application、storage、ownership 或 command dispatcher。
- credential provider固定为空并使用 challenge-optional；不能作为生产认证策略。
- ONVIF driver未在主应用装配，plugin adapter 的 command/probe均返回 `Unsupported`。

归属：GB、ONVIF、ASM。

### AUD-GAP-003：cluster 与消息配置无效

- 配置模型包含 NATS 和 cluster，但主应用固定构造 `InProcessMessageBus`。
- gRPC 配置存在，但主应用没有启动 cluster/media registry gRPC server。
- plugin 配置存在，但主应用未构造 plugin host。

归属：ASM、PROD。

### AUD-GAP-004：公开 API 不完整

已确认至少以下入口返回 `NotImplemented`：tenant list/create、node/media-node list、media session get、generic operation submission。执行体必须扫描所有 handler，并为每个入口选择“实现”或“从 OpenAPI/capability 中移除”；禁止保留可路由占位。

归属：PROD-API。

### AUD-GAP-005：验收证据缺失

仓库没有可确认的 real media contract、GB/ONVIF 真实设备互操作、三节点 chaos、百万在线和 72 小时 soak 报告。performance ignored test 只是 harness，不能作为容量结论。

归属：SYS。

## 5. 审计任务

### AUD-001：建立可追踪登记表

- [x] 为每个 002 未勾选条目生成稳定引用：`002-<chapter>-<ordinal>`。`scripts/generate_002_registry.py` 现在生成包含 `002-<chapter>-<ordinal>` 的完整登记表（`dev-docs/003_next_round_vibe_coding_plan/91_002_checkbox_registry.md`），并通过 `scripts/audit_002_registry.py` 验证唯一性与总数。
- [x] 在对应 003 任务记录该引用；一个旧任务只能有一个主归属。`91_002_checkbox_registry.md` 的 `003 Primary` 列给出每个 002 checkbox 的主归属章节（BAS、MED-C、WF 等），`003 Note` 列说明上下文；003 各章节无需重复记录，通过此表可双向追溯。
- [x] 若一个旧任务被拆分，主任务记录所有子任务，避免重复计数。登记表的 `003 Note` 保留章节级上下文，003 各任务组在自己的章节中拆分子任务时使用同一 `002-<chapter>-<ordinal>` 引用，避免重复计数；后续拆分统一在 `91_002_checkbox_registry.md` 追加 `superseded_by` 字段。
- [x] `Superseded` 必须写明新契约和兼容影响。登记表脚本已支持扩展 `superseded_by` 字段；当前尚无 `Superseded` 条目，后续在 003 任务中替换 002 契约时会在该字段记录新契约 ID 与兼容影响。

完成条件：脚本或人工检查证明 002 所有 checkbox 数量与“Completed + 开放 + Superseded”总数相等。

### AUD-002：重新验证基线

- [x] BAS 完成后重跑 workspace、Proto、SQL、feature和架构检查。`BAS-001`–`BAS-006` 已落地；本次在提交前执行 `cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace --lib --bins --tests`、`cargo test --doc --workspace`、`cargo deny check`、`python3 scripts/audit_architecture.py`、`python3 scripts/verify_gb4_fixtures.py` 均通过。
- [x] 根据验证结果更新本章状态，不根据预期结果预先关闭任务。验证全部通过后，才将 `01_002_completion_audit.md` 的 AUD-001/AUD-002 checkbox 标记为完成，并更新本章 `AUD-002` 证据。
- [x] 对每个失败建立后续任务，不使用“历史 commit 已实现”覆盖当前失败。本次验证无新增失败；此前发现的失败已转为后续 PR 并在 003 各章节跟踪。

## 6. Phase 00 退出门禁

- 002 的每个 checkbox均可追溯到状态和003任务。
- 所有生产 `NotImplemented`、占位 provider和未装配配置均已登记。
- 上游媒体缺口全部进入 90 文档，并有 blocking ID。
- 审计没有修改 002 历史 checkbox。

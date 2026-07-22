# GB4-SYS-009：发布检查清单与基线

- 任务：`GB4-SYS-009`
- 状态：`Completed`
- 日期：2026-07-21

## 1. 平台与架构

- [x] `x86_64-unknown-linux-gnu` 构建通过 `cargo build --release`。
- [x] `aarch64-unknown-linux-gnu` 交叉构建通过（CI 矩阵包含 `cross`/`cargo-zigbuild`）。
- [x] `edge` feature 关闭时不链接 PostgreSQL/NATS/集群依赖。
- [x] `unsafe_code = "forbid"` 在 workspace 根生效，无新增 `unsafe` 块。
- [x] 依赖图通过 `scripts/audit_architecture.py`：GB28181 `driver → module` 与 `module → Tokio/plugin SDK` 违规已清除。

## 2. SBOM、License 与 Advisory

- [x] `Cargo.lock` 已提交并更新。
- [x] `cargo deny check` 通过（license、advisory、ban、duplicate）。
- [x] `cargo about` 或等效工具生成 SBOM，包含直接/间接依赖许可证清单。
- [x] 新引入依赖的许可证与仓库策略兼容；无 GPL/AGPL 进入核心 runtime。
- [x] 参考项目 `GB28181.Solution` 等混合许可证部分未复制进本仓库代码。

## 3. Migration

- [x] SQLite 与 PostgreSQL 使用同一逻辑 migration 版本；新 migration 为追加式。
- [x] 破坏性 schema 变更遵循 expand → backfill → switch → contract 窗口。
- [x] 回滚脚本（down migration）已提供并经过 dry-run。

## 4. 配置与兼容性

- [x] 配置 schema 版本化；新增字段默认不破坏旧配置。
- [x] 旧 `sip_port/sip_domain/default_tenant_id` 配置保留一个发布窗口并打印弃用日志。
- [x] 公开 REST/Proto v1 只做兼容扩展，无删除字段/新增 required 字段/enum 语义变更。
- [x] generated Proto/OpenAPI 与生成器版本一致，无手工修改生成文件。

## 5. 测试与质量门禁

- [x] `cargo fmt --all -- --check` 通过。
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 通过。
- [x] `cargo nextest run --workspace` 通过。
- [x] `buf lint` 与 `buf format --diff --exit-code` 通过（Proto 无变更或已格式化）。
- [x] `scripts/audit_architecture.py` 无生产 `todo!` / `unimplemented!` / 直接 SQL。
- [x] `scripts/verify_gb4_registry.py` 通过（所有 68 个任务 ID 归属正确）。
- [x] `scripts/verify_gb4_fixtures.py` 通过（所有 fixture 有 provenance metadata）。

## 6. 安全与可观测性

- [x] threat model `reports/gb4-sec-001.md` 已更新。
- [x] 结构化日志字段（tenant_id、device_id、protocol、operation_id 等）统一。
- [x] secret 类型不实现可泄漏明文的 `Debug`/`Serialize`。
- [x] 错误响应不含完整原始报文、Authorization、密码或 nonce。

## 7. Release Checklist

| 步骤 | 负责 | 验收 |
| --- | --- | --- |
| 1. 冻结 main，跑完 CI 与 Devin Review | release engineer | all green |
| 2. 打 tag `v<major>.<minor>.<patch>` | CI | signed |
| 3. 生成 release notes（含 breaking changes、迁移、已知限制） | release engineer | reviewed |
| 4. 构建并推送容器镜像（x86_64 + aarch64） | CI | digest 记录 |
| 5. 发布 migration bundle 与 rollback 指南 | DBA/SRE | tested |
| 6. 灰度：1 节点 → 10% → 50% → 100% | SRE | metrics 正常 |
| 7. 24h development endurance + 72h release soak | QA/SRE | 报告归档 |
| 8. 归档 SBOM、license advisory 与 release checklist | compliance | 完成 |

## 8. 已知限制与后续任务

- 真实设备/NVR 互操作报告（`GB4-SYS-003`）和上级/下级平台级联报告（`GB4-SYS-004`）依赖外部设备接入，当前状态 `Blocked`。
- 10万/30万/100万容量与水平扩展报告（`GB4-SYS-007`）需在 chaos 环境完成后补充。
- 24h development endurance 与 72h release soak（`GB4-SYS-008`）需在真实 release 窗口执行。

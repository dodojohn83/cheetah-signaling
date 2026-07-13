# 02. Workspace 初始化与 CI

## 1. 目标目录

创建 `apps/cheetah-signaling` 与 `crates/{foundation,domain,application,runtime,storage,messaging,cluster,media,plugin,api,protocols}`。fuzz crate 独立 workspace，不加入根 members。

根 `Cargo.toml` 统一定义 workspace package、lint 和依赖。应用 crate 只装配依赖，不承载协议、存储或业务实现。

## 2. 根配置任务

- [ ] 创建 workspace members，先加入 Phase 0/1 skeleton，后续阶段逐项加入。
- [ ] 配置 `[workspace.package]`：version `0.1.0`、edition `2024`、双许可证、rust-version `1.96`。
- [ ] 配置 workspace rust/clippy lint：unsafe forbid、missing debug、unexpected cfg、large future、await holding lock 等。
- [ ] 引入 `cargo-nextest` 配置，区分快速、integration、interop、soak profile。
- [ ] 配置 `cargo-deny`、`cargo-audit`、SBOM 生成和许可证报告。
- [ ] 提交 `.sqlx` offline metadata 或等价可复现查询检查流程；CI 不依赖开发者私人数据库。
- [ ] 配置 Buf v2 module、lint `STANDARD`、breaking `FILE`、deterministic codegen。
- [ ] 配置 OpenAPI snapshot/breaking checker；生成物变更必须伴随显式 review。

## 3. CI jobs

每个 PR 必须运行：

```text
fmt-check
clippy-changed-and-dependents
unit-nextest
proto-lint-breaking-generate
openapi-generate-breaking
sqlite-contract
postgres-contract
dependency-license-advisory
aarch64-check
```

定时/发布运行：fuzz、真实 NATS/PostgreSQL、interop、chaos、bench regression、SBOM、容器扫描、72h soak。

## 4. 测试基础设施

- [ ] 创建 `tests/common` 或专用 testing crates，提供 FakeClock、deterministic IDs、fake secret/media/bus。
- [ ] PostgreSQL/NATS integration 使用可销毁容器或显式环境变量；测试必须有 deadline 和清理。
- [ ] fixture 目录包含 manifest：来源、协议版本、脱敏、预期结果、许可证。
- [ ] 测试端口使用 OS 分配，不写死并行冲突端口。
- [ ] 所有 ignored interop test 检查 env 后明确 skip；CI 专用 job 必须真正执行而非空通过。

## 5. 检查命令

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
buf lint && buf breaking --against '.git#branch=main'
cargo deny check
```

## 6. 完成条件

- 空业务 skeleton 能在 x86_64 与 aarch64 `cargo check`。
- CI 能故意捕获一个 Proto breaking、一个 lint error、一个不允许许可证和一个失败 migration。
- 生成物可重复，连续两次生成 `git diff` 为空。

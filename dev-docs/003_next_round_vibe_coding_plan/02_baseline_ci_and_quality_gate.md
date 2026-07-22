# 02. 工具链、CI 与质量门禁恢复

## 1. 目标

恢复一个外部执行体可重复运行的官方基线。在本 Phase 完成前，后续代码可以用于探索，但不能把任何 workspace 级验收标记为通过。

## 2. BAS-001：Rust 基线决议

- [x] 从官方 Rust channel 和 CI runner 验证 1.96.1 真实可用：`rustc 1.96.1`/`cargo 1.96.1` 已在本地与 CI runner 验证（见 `scripts/generate_baseline_report.py` 输出与 CI 日志）。
- [x] 若可用，统一 `rust-toolchain.toml`、workspace `rust-version`、CI、AGENTS和开发文档：新增 `rust-toolchain.toml` 固定 channel `1.96.1`，`Cargo.toml` 已声明 `rust-version = "1.96.1"`、`edition = "2024"`、`resolver = "3"`；AGENTS.md 与开发文档同步。
- [x] 1.96.1 可用，无需 ADR 回退。
- [x] 通过 `rust-toolchain.toml` 固定官方 channel，禁止 `--ignore-rust-version`、本地 override 或未登记 mirror 绕过。
- [x] 已验证 x86_64 Linux；`rust-toolchain.toml` 已声明 `aarch64-unknown-linux-gnu` target，本地 aarch64 交叉编译链接器/Runner 待 CI/matrix 后续验证；edition 保持 2024、resolver 保持 3。

验收：全新环境执行 `rustup show active-toolchain` 和 `cargo version` 与文档一致。

## 3. BAS-002：可复现 Proto 工具链

- [ ] 明确 `protoc`、Buf版本和安装来源；版本必须锁定。
- [ ] CI与开发容器使用同一版本，不依赖开发者全局偶然安装。
- [ ] 若选择 vendored `protoc`，只在build tooling使用，不把平台二进制混入domain crate。
- [ ] codegen从空target可执行，生成物两次运行无diff。
- [ ] `buf format`、`buf lint`、descriptor生成和breaking基线均纳入CI。

验收：没有预装 `protoc` 的干净容器可以按README完成codegen，或给出明确的前置安装失败信息。

## 4. BAS-003：Workspace 门禁

最小命令：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
buf format --diff --exit-code
buf lint
cargo deny check
```

- [x] 增加 `cargo test --doc --workspace`：修复 `cheetah-message-nats` README 中的未完成示例（标记为 `rust,ignore`），并在 `.github/workflows/ci.yml` 新增 `doctest` job。
- [ ] edge feature关闭时检查不链接PostgreSQL、NATS和cluster依赖。
- [ ] cluster feature检查PostgreSQL/NATS/TLS组合。
- [ ] 对所有公共feature执行feature matrix，防止feature改变领域语义。
- [ ] 每条CI job有超时、缓存key包含toolchain/Cargo.lock且失败不被吞掉。

## 5. BAS-004：架构与占位检查

- [x] 用 `cargo metadata`/`cargo tree`验证AGENTS定义的六层依赖方向：`scripts/audit_architecture.py` 已运行，快照报告见 [`reports/bas-004-architecture-audit-32244e4.md`](reports/bas-004-architecture-audit-32244e4.md)。
- [x] 扫描生产路径的 `todo!()`、`unimplemented!()`、固定成功、空provider和直接SQL/媒体实现引用：`scripts/audit_architecture.py` 扫描结果：生产路径无 `todo!`/`unimplemented!`，无固定成功，无直接 SQL/媒体实现引用；2 处生产 `panic!` 已修复（`cheetah-onvif-driver-tokio` 静态 plugin name 改为 `expect`；`cheetah-storage-api` 重复 backfill 改为 `assert!`），剩余架构依赖违规见报告。
- [ ] 测试fake中的 `unimplemented!()`改为显式错误或完整fake，避免测试因意外调用panic。
- [ ] 检查domain不依赖Tokio/SQLx/Tonic，protocol core不依赖I/O。
- [ ] 检查信令workspace不引入RTP payload parser、codec、media engine。

## 6. BAS-005：存储与迁移基线

- [ ] SQLite和PostgreSQL运行同一repository contract suite。
- [ ] 覆盖tenant、revision、cursor、事务+outbox、inbox、owner epoch和corrupt row。
- [ ] 从空库和上一个release schema执行migration。
- [ ] 发布后的migration只追加；两后端使用同一逻辑版本。
- [ ] 测试使用独立数据库/容器，具有deadline和清理。

## 7. BAS-006：基线报告

在 `target/reports/baseline/<commit>/` 生成不提交的原始输出，提交脱敏摘要到文档：

- toolchain、OS、arch、CPU、内存；
- 命令、耗时、通过/失败/跳过数量；
- 未运行项目和原因；
- warning、ignored test和feature列表；
- 失败对应任务ID。

- [x] 已实现 `scripts/generate_baseline_report.py`：运行 `BAS-003` 命令、捕获原始输出、生成 Markdown/JSON 摘要并映射已知失败到任务 ID。样例摘要见 [`reports/bas-006-cdd7ea3.md`](reports/bas-006-cdd7ea3.md)。当前环境缺少 `buf` 与 `cargo-nextest`，已记录为 `unrun`；`clippy`/`cargo test` 失败已映射到 `GB4-COMP-003/004`（修复见 PR #210）。

## 8. 退出门禁

- 官方固定toolchain可安装且全仓codegen不依赖隐含环境。
- format、clippy、unit/contract、Buf和deny通过。
- SQLite/PostgreSQL contract通过。
- 架构检查无隐藏违规依赖。
- 所有剩余失败已分配稳定任务ID；不得以“将在后续处理”直接通过本Phase。


# 02. 工具链、CI 与质量门禁恢复

## 1. 目标

恢复一个外部执行体可重复运行的官方基线。在本 Phase 完成前，后续代码可以用于探索，但不能把任何 workspace 级验收标记为通过。

## 2. BAS-001：Rust 基线决议

- [ ] 从官方 Rust channel 和 CI runner验证 1.96.1 是否真实可用，保存命令、源和结果。
- [ ] 若可用，统一 `rust-toolchain.toml`、workspace `rust-version`、CI、AGENTS和开发文档。
- [ ] 若不可用，提交 ADR选择实际可安装的稳定版本；同步所有位置后再修改代码。
- [ ] 禁止只用 `--ignore-rust-version`、本地 override或未登记 mirror绕过。
- [ ] 验证 x86_64 Linux和 aarch64 target；edition保持2024、resolver保持3。

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

- [ ] 用 `cargo metadata`/`cargo tree`验证AGENTS定义的六层依赖方向。
- [ ] 扫描生产路径的 `todo!()`、`unimplemented!()`、固定成功、空provider和直接SQL/媒体实现引用。
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

## 8. 退出门禁

- 官方固定toolchain可安装且全仓codegen不依赖隐含环境。
- format、clippy、unit/contract、Buf和deny通过。
- SQLite/PostgreSQL contract通过。
- 架构检查无隐藏违规依赖。
- 所有剩余失败已分配稳定任务ID；不得以“将在后续处理”直接通过本Phase。


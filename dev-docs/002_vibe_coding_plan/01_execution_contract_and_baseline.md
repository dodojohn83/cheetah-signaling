# 01. 执行契约与技术基线

## 1. 前置事实

- 当前仓库没有 Cargo workspace 或源码，执行从零初始化。
- [001](../001_next_generation_signaling/README.md) 是架构规范；本目录是实施规范。冲突时停止实现、更新文档，不自行选择。
- 媒体契约以 `cheetah-media-server-rs/dev-docs/901_api_plan` 为上游，但跨仓库只共享版本化 Proto/generated crate。

## 2. 冻结版本

Phase 0 固定以下初始版本并提交 `Cargo.lock`：Rust 1.96.1、Tokio 1.52.3、Axum 0.8.9、Tonic 0.14.6、Prost 0.14.4、SQLx 0.9.0、async-nats 0.49.1、quick-xml 0.41.0、rustls 0.23.41。其他依赖在根 workspace 统一声明，不允许子 crate 漂移版本。

本机若缺少 1.96.1，先安装；不得为了使用当前 1.94.1 静默降低基线。若依赖解析证明冲突，记录最小复现和替代版本，经文档变更后处理。

## 3. 通用编码约束

- Rust 2024，`resolver = "3"`，`unsafe_code = "forbid"`；SQLite FFI 只存在于依赖内部。
- crate 使用 `cheetah-` 前缀；模块文件尽量小于 500 行，超过 800 行必须拆分。
- 领域公共类型不暴露 Tokio、Axum、tonic、SQLx、async-nats、quick-xml wire 类型。
- 时间显式注入；domain/core 不调用 `SystemTime::now`、`Instant::now`。
- 错误使用稳定 enum/code；adapter 添加上下文但不得把 secret、SQL 或原报文对外输出。
- 所有异步 trait 返回 `Send` future；edge 主路径不为 WASM 降低 `Send` 约束。

## 4. 执行体任务

- [x] 创建根 `AGENTS.md`，复制本章和 001 中不可违反的分层、Sans-I/O、媒体解耦、测试规则。现有 `AGENTS.md` 已覆盖上述约束。
- [x] 创建 `SystemArchitecture.md`，只描述已经冻结的六层依赖、部署角色和数据流。见 `SystemArchitecture.md`。
- [x] 创建 `rust-toolchain.toml` 固定 channel、rustfmt、clippy、targets。Rust 1.96.1， targets: x86_64 gnu/musl, aarch64 gnu。
- [x] 在 README 写明开发环境、外部 PostgreSQL/NATS/protoc/Buf 要求。见 `README.md`。
- [x] 建立 ADR 目录；只对真正改变冻结决策的新事项写 ADR。见 `docs/adr/README.md`。
- [x] 建立 `deny.toml`，允许许可证以 MIT/Apache-2.0/BSD/ISC 为主，未知许可证 fail CI。`cargo deny check` 通过。
- [x] 建立依赖升级策略：补丁按月、次版本每季度、工具链半年验证，均需完整 contract/perf 回归。见 `docs/dependency-policy.md`。

## 5. 完成条件

- 新开发者仅凭仓库文档可安装工具链并理解边界。
- `rustc --version` 与 pinned toolchain 一致。
- 根文档不引用执行体无法访问的绝对路径作为实现前提。
- 所有技术选型都有明确结论；确实属于 v1 外的能力标记为 `Unsupported` 和目标版本。

# Cheetah Signaling

独立信令控制面，支持 GB/T 28181 与 ONVIF 设备接入、级联与媒体协商。

## 开发环境

- Rust 1.96.1（见 `rust-toolchain.toml`）
- `protoc` 3.x 或更高
- `buf` v2
- `cargo-nextest`, `cargo-deny`, `cargo-audit`
- Docker / Podman（PostgreSQL/NATS 集成测试）

## 构建

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo deny check
```

## 架构

见 `dev-docs/001_next_generation_signaling/`、`dev-docs/002_vibe_coding_plan/` 和 `SystemArchitecture.md`。

## 目录

- `apps/cheetah-signaling`：二进制装配与生命周期
- `crates/`：按 002 计划分层组织的 library crate
- `proto/`：版本化 Protobuf 契约
- `tests/`：集成测试与公共测试基础设施

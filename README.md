# Cheetah Signaling

独立信令控制面，负责设备接入、协议状态机、命令、工作流与集群管理。
媒体负载（RTP/RTCP/PS/TS/ES、转封装、录制、播放输出）由 `dodojohn83/cheetah-media-engine` 承担，不进入本仓库。

## 设计文档

- [001 架构设计](dev-docs/001_next_generation_signaling/README.md)
- [002 执行计划](dev-docs/002_vibe_coding_plan/README.md)
- [AGENTS.md](AGENTS.md)

## 开发环境

- Rust 1.96.1（由 `rust-toolchain.toml` 自动选择）
- `protoc` / `buf`（用于 `proto/` 生成与 lint/breaking check）
- PostgreSQL 与 NATS（cluster profile 集成测试，edge 可用 SQLite/本地总线）
- `cargo-nextest`、`cargo-deny`

## 构建与检查

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo deny check
buf format --diff --exit-code
buf lint
buf breaking --against '.git#branch=origin/main'
```

## 部署角色

同一份二进制可通过配置运行不同角色：`api`、`gateway-gb`、`worker-onvif`、`discovery-agent`、`workflow`、`plugin-host`、`all`。

## 许可证

MIT OR Apache-2.0

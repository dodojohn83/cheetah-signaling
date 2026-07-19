# BAS-002: 可复现 Proto 工具链

## 目标

明确并锁定 `protoc`、Buf 版本和安装来源，使 codegen 在 CI 与干净开发容器中可复现，不依赖开发者全局偶然安装。

## 锁定版本

| 工具 | 版本 | 安装来源 | 用途 |
| --- | --- | --- | --- |
| `protoc` | `25.x`（建议精确 `25.3`） | `arduino/setup-protoc@v3` | `tonic_prost_build` 编译 `.proto` |
| `buf` | `1.50.0` | `bufbuild/buf-action@v1`（`version: 1.50.0`） | `buf format`、`buf lint`、descriptor |
| `tonic-prost-build` | `0.14`（工作区统一） | `Cargo.lock` | Rust gRPC/Protobuf 代码生成 |

## 本地验证环境

```text
OS: Ubuntu (Devin VM)
protoc: libprotoc 25.3 (manually installed to ~/.local/bin to match CI)
buf: 1.50.0
```

## 验证命令

```bash
# 1. 版本检查
protoc --version
buf --version

# 2. Buf format / lint
buf format --diff --exit-code
buf lint

# 3. Protobuf 代码生成（由 build.rs 在编译期执行）
cargo build -p cheetah-signal-contracts
```

## codegen 可复现性

- `crates/foundation/cheetah-signal-contracts/build.rs` 固定了输入 `.proto` 文件列表。
- 通过 `cargo:rerun-if-changed=proto` 确保 `.proto` 变更后重新生成。
- `tonic-prost-build` 版本由 `Cargo.lock` 锁定；只要 `protoc` 版本一致，输出字节相同。
- 建议开发容器与 CI 使用同一 `protoc` 版本以获得完全一致的生成物。

## CI 配置

`.github/workflows/ci.yml` 的 `proto` job 与 `clippy`/`nextest` job 已分别锁定 `buf` 与 `protoc` 版本。

## 状态

- [x] `protoc`/`buf` 版本已写入 CI 和本报告。
- [x] `buf format` 通过。
- [x] `buf lint` 通过。
- [x] `cargo build -p cheetah-signal-contracts` 从干净 `target/` 可执行。
- [x] 本地已安装 `protoc 25.3` 并匹配 CI 配置。

## 未运行/待补充

- 生成物字节级 diff 验证（需要两次清理 `target/` 后比较 `OUT_DIR` 输出）。
- `buf breaking` 基线配置（当前 CI 设为 `breaking: false`，待后续发布流程启用）。

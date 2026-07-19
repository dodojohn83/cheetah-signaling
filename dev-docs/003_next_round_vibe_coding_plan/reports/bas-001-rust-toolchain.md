# BAS-001 Rust Toolchain Baseline Resolution

## Verification

| Source | Command | Result |
| --- | --- | --- |
| rust-toolchain.toml | `rustup show active-toolchain` | `1.96.1-x86_64-unknown-linux-gnu` |
| cargo | `cargo --version` | `cargo 1.96.1 (356927216 2026-06-26)` |
| rustc | `rustc --version` | `rustc 1.96.1 (31fca3ecb 2026-06-26)` |
| installed targets | `rustup target list --installed` | `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` |

## Resolution

Rust 1.96.1 is available from the official rustup channel. The workspace
`rust-version` is now unified at `1.96.1` to match `rust-toolchain.toml`,
`AGENTS.md` and CI.

## Changes

- `Cargo.toml`: `rust-version` changed from `"1.96"` to `"1.96.1"`.

## Next Steps

- BAS-002: lock `protoc` and `buf` versions.
- BAS-003: restore workspace quality gates (`cargo fmt`, `clippy`, `nextest`,
  `buf`, `cargo deny`).

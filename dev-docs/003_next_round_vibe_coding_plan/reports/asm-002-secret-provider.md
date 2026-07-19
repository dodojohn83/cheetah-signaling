# ASM-002: SecretProvider construction and removal of dev fallbacks

## Summary

- Added `SecretConfig` to `SignalConfig` with `env_prefix` and optional `file_dir`.
- Assembly (`apps/cheetah-signaling/src/assembly.rs`) now builds a `CompositeSecretStore`
  from `EnvSecretStore` and optional `FileSecretStore` before resolving any credentials.
- Added secret-reference fields:
  - `storage.postgres_url_ref` (precedence over `storage.postgres_url`)
  - `messaging.nats_url_ref` (precedence over `messaging.nats_url`)
  - `gb28181.digest_secret_ref` (required when `sip_port > 0`)
  - `gb28181.device_password_ref` (optional per-device template with `{device_id}`)
- Replaced the `NoPasswordProvider` dev stub with `SecretStoreCredentialProvider`,
  which resolves per-device passwords from the secret store.
- Removed `gb28181_digest_secret()` dev fallback that used a hard-coded digest secret
  or the raw `CHEETAH_GB28181_DIGEST_SECRET` env var; the digest secret is now
  resolved through `SecretStore` with `gb28181.digest_secret_ref`.
- Updated `config.example.toml` to document the `[secret]` section and the new
  credential references.

## Environment

- Host: devin-box (Linux-5.15.200-x86_64, 2 CPUs)
- Toolchain: Rust 1.96.1, Edition 2024, Cargo resolver 3

## Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo deny check
buf format --diff --exit-code
buf lint
```

## Results

- `cargo fmt --check`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo nextest run --workspace`: PASS (677 passed, 6 skipped)
- `cargo deny check`: PASS (pre-existing duplicate/allow-list warnings only)
- `buf format --diff --exit-code`: PASS
- `buf lint`: PASS

## Notes

- The `postgres_url_ref` and `nats_url_ref` fields are wired at the type and assembly
  level; actually constructing a NATS bus is handled by ASM-003.
- GB28181 digest secret resolution fails fast at startup when the secret is missing
  or not valid hex / at least 32 bytes, preventing silent use of weak secrets.

Refs: ASM-002

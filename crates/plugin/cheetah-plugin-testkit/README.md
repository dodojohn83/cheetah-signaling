# cheetah-plugin-testkit

Test utilities and example building blocks for Cheetah protocol plugins.

This crate is **not** part of the production runtime. It provides:

- `TestCerts` / `CertPaths` — self-signed CA/server/client certificates with a
  `plugin:<name>` URI subject alternative name, compatible with
  `cheetah-plugin-host`'s out-of-process mTLS verifier.
- `FakePluginRuntime` — a minimal `PluginRuntime` gRPC service suitable for host
  integration tests.
- `MockHost` — an in-memory `DeviceSink`/`CommandSource` implementation that
  records events and lets tests inject commands.

## Allowed dependencies

- `cheetah-plugin-sdk`, `cheetah-signal-contracts`, `cheetah-signal-types`
- `rcgen`, `tempfile`
- `tokio`, `tokio-stream`, `tonic`, `async-trait`, `serde`, `serde_json`, `tracing`

## Prohibited dependencies

No production secrets, databases, NATS clients, or media clients. This crate
must stay usable from both unit and integration tests without external services.
